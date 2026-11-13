//! Selection and execution share the same dependencies, independently of any UI adapter.
use super::*;
use td_link::update::{self, UpdateState};

/// Which Firmware write to perform. Job names describe output; they never select behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Update,
    Repair,
    Install,
}

impl Firmware {
    /// The normal and optional radio builds carried by this Companion.
    pub fn builds(&self) -> Result<FirmwareBuild, FirmwareError> {
        let b = self.bundled(FirmwareChannel::Normal)?;
        Ok(FirmwareBuild {
            version: b.manifest.version.clone(),
            build: b.manifest.elf_sha256.clone(),
            radio: self
                .bundled(FirmwareChannel::Radio)
                .ok()
                .map(|r| RadioBuild {
                    version: r.manifest.version.clone(),
                    build: r.manifest.elf_sha256.clone(),
                }),
        })
    }

    /// Opens the native picker and binds the selection to this target for ten minutes.
    /// A selection is consumed only when a Job starts, including one later declined or cancelled.
    pub async fn pick(&self, target: DeviceId) -> Result<PickResult, FirmwareError> {
        self.target(&target, false)?;
        if self.dialog.swap(true, Ordering::SeqCst) {
            return Err(FirmwareError::DialogOpen);
        }
        let guard = Dialog(self.dialog.clone());
        let prompt = self.prompter.lock().clone();
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let _input = service.devices.suppress_input();
            let Some(bytes) = prompt.pick().map_err(|_| FirmwareError::Invalid)? else {
                return Ok(PickResult { bundle: None });
            };
            let bundle = Arc::new(Bundle::parse(&bytes).map_err(|_| FirmwareError::Invalid)?);
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|_| CommonError::Internal)?;
            let id = random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let info = BundleInfo {
                id: id.clone(),
                version: bundle.manifest.version.clone(),
                build: bundle.manifest.elf_sha256.clone(),
                bytes: bundle.app().len(),
                channel: FirmwareChannel::of(bundle.manifest.radio),
            };
            let mut picked = service.picked.lock();
            picked.retain(|_, p| p.until > service.clock.now());
            if picked.len() >= 16 {
                return Err(CommonError::Limits("Too many selected Firmware files.").into());
            }
            picked.insert(
                id,
                Picked {
                    target,
                    until: service.clock.now() + Duration::from_secs(600),
                    bundle,
                },
            );
            Ok(PickResult { bundle: Some(info) })
        })
        .await
        .map_err(|_| CommonError::Internal)?
    }

    /// Starts an owned workflow and returns its Job immediately. Observe progress and completion
    /// through `Handle::observe` / `Handle::jobs`; use `Handle::jobs().cancel` while cancellable.
    /// Requires a Tokio runtime; work runs on a blocking worker. Dropping a UI handle does not
    /// cancel work. Consent is always supplied by the PC prompter.
    pub fn start(
        &self,
        operation: Operation,
        id: DeviceId,
        request: FirmwareRequest,
    ) -> Result<Job, FirmwareError> {
        match operation {
            Operation::Update => self.start_job::<Update>(operation, id, request),
            Operation::Repair => self.start_job::<Repair>(operation, id, request),
            Operation::Install => self.start_job::<Install>(operation, id, request),
        }
    }

    fn target(
        &self,
        id: &DeviceId,
        install: bool,
    ) -> Result<(String, String, String), FirmwareError> {
        if !install && let Some(d) = self.devices.get(id) {
            if d.carrier == Some(crate::devices::Carrier::Wifi) {
                return Err(FirmwareError::UsbOnly);
            }
            let port = self
                .devices
                .ports()
                .into_iter()
                .find(|p| &p.id == id)
                .ok_or(crate::link::DeviceLinkError::NotFound)?
                .port;
            return Ok((
                d.name.unwrap_or_else(|| id.to_string()),
                port,
                d.firmware
                    .map(|f| f.version)
                    .unwrap_or_else(|| "unknown".into()),
            ));
        }
        self.devices
            .new_boards()
            .into_iter()
            .find(|b| &b.id == id)
            .map(|b| (id.to_string(), b.port, "unknown".into()))
            .ok_or(if install {
                FirmwareError::NewBoardNotFound
            } else {
                crate::link::DeviceLinkError::NotFound.into()
            })
    }
    fn start_job<K: JobKind<Progress = FirmwareProgress, Result = FirmwareResult>>(
        &self,
        operation: Operation,
        id: DeviceId,
        request: FirmwareRequest,
    ) -> Result<Job, FirmwareError> {
        let FirmwareRequest {
            bundle: selection,
            channel: requested,
        } = request;
        let install = operation == Operation::Install;
        let recovery = operation != Operation::Update;
        let (name, port, current) = self.target(&id, install)?;
        // The build the Device runs, from its last hello. A Device under repair may have sent none, so its build is
        // unknown; an update needs a linked Device, which has always sent one.
        let running = if install {
            None
        } else {
            let device = self
                .devices
                .get(&id)
                .ok_or(crate::link::DeviceLinkError::NotFound)?;
            device
                .firmware
                .map(|f| f.channel)
                .or((!recovery).then(FirmwareChannel::default))
        };
        // The Device's own build picks the bundle, and an unknown build gets the normal one. Only a request that names
        // the other build switches.
        let channel = requested.or(running).unwrap_or_default();
        let switching = running.is_some_and(|running| running != channel);

        let mut picked = self.picked.lock();
        let bundle = if selection == "bundled" {
            self.bundled(channel)?
        } else {
            picked
                .get(&selection)
                .filter(|p| p.target == id && p.until > self.clock.now())
                .ok_or(FirmwareError::Missing)?
                .bundle
                .clone()
        };
        let built = FirmwareChannel::of(bundle.manifest.radio);
        if built != channel {
            return Err(FirmwareError::ChannelMismatch {
                channel,
                bundle: built,
            });
        }
        let consent = recovery || switching || !same_or_newer(&bundle.manifest.version, &current);
        let question = question(
            install,
            recovery,
            running,
            channel,
            older(&bundle.manifest.version, &current),
        );
        let confirmation = Confirmation {
            question,
            target: id.to_string(),
            name,
            port,
            current,
            version: bundle.manifest.version.clone(),
            build: bundle.manifest.elf_sha256.clone(),
            channel,
        };
        let jobs = self.jobs.clone();
        let scope = if install {
            JobScope::NewBoard(id.to_string())
        } else {
            JobScope::Device(id.to_string())
        };
        if let Some(job) = jobs.holding(&scope) {
            return Err(JobError::Busy(Box::new(job)).into());
        }
        if !recovery {
            self.links.connected(&id)?;
        }
        let work = self.clone();
        let job = jobs.start::<K>(scope, true, move |job| {
            let hold = work
                .devices
                .firmware_hold(&id, job.id())
                .map_err(|_| FirmwareError::Failed("Target disconnected.".into()))?;
            if consent {
                job.waiting_for_confirmation();
                let _input = work.devices.suppress_input();
                let prompt = work.prompter.lock().clone();
                let deadline = work.clock.now() + CONSENT_WAIT;
                let expired = || work.clock.now() >= deadline;
                let accepted = prompt.confirm(&confirmation, &|| expired() || job.check().is_err());
                job.check()?;
                if !accepted || expired() {
                    return Err(FirmwareError::Declined.into());
                }
            }
            job.set_cancellable(false);
            job.check()?;
            job.running();
            let mut emit = |s| {
                let (phase, pct) = match s {
                    UpdateState::Checking => ("checking", None),
                    UpdateState::Sending { pct } => ("sending", Some(pct)),
                    UpdateState::Rebooting => ("rebooting", None),
                    UpdateState::Confirming => ("confirming", None),
                    UpdateState::Recovering { pct } => ("recovering", Some(pct)),
                    _ => return,
                };
                job.progress(&FirmwareProgress {
                    phase: phase.into(),
                    pct,
                    bytes: pct.map_or(0, |p| {
                        let total = if recovery {
                            bundle.images.iter().map(|image| image.data.len()).sum()
                        } else {
                            bundle.app().len()
                        };
                        total * usize::from(p) / 100
                    }),
                });
            };
            let mut connector = hold.connector();
            let result = if recovery {
                if let Some(s) = &hold.session {
                    s.close();
                }
                #[cfg(any(test, feature = "test-hooks"))]
                if let Some((ports, _)) = work.devices.mock() {
                    let deck = ports
                        .deck(&id)
                        .ok_or_else(|| FirmwareError::Failed("Target disconnected.".into()))?;
                    deck.flash_bundle(&bundle);
                    emit(UpdateState::Recovering { pct: 100 });
                    if install {
                        hold.installed();
                        work.skips.unskip(&id);
                    }
                    return Ok(FirmwareResult {
                        message: "Firmware installed".into(),
                        version: bundle.manifest.version.clone(),
                    });
                }
                let mut flasher = td_link::recover::RomRecovery(&hold.port);
                update::run(
                    &mut *connector,
                    Some(&mut flasher),
                    &bundle,
                    update::Deadlines::default(),
                    &mut emit,
                )
            } else {
                let session = hold
                    .session
                    .as_ref()
                    .ok_or_else(|| FirmwareError::Failed("Target disconnected.".into()))?;
                update::run_session(
                    session,
                    &mut *connector,
                    &bundle,
                    update::Deadlines::default(),
                    &mut emit,
                )
            };
            result.map_err(FirmwareError::Failed)?;
            if install {
                hold.installed();
                work.skips.unskip(&id);
            }
            Ok(FirmwareResult {
                message: "Firmware updated".into(),
                version: bundle.manifest.version.clone(),
            })
        })?;
        if selection != "bundled" {
            picked.remove(&selection);
        }
        Ok(job)
    }
}

fn versions(next: &str, current: &str) -> Option<(semver::Version, semver::Version)> {
    Some((
        semver::Version::parse(next.trim_start_matches('v')).ok()?,
        semver::Version::parse(current.trim_start_matches('v')).ok()?,
    ))
}
pub(super) fn same_or_newer(next: &str, current: &str) -> bool {
    next == current || versions(next, current).is_some_and(|(n, c)| n >= c)
}
pub(super) fn older(next: &str, current: &str) -> bool {
    versions(next, current).is_some_and(|(n, c)| n < c)
}
