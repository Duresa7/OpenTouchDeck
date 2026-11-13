//! About's facts: the version and the licences of the components the program uses directly.
use crate::{AppWindow, Component};
use slint::{ModelRc, VecModel};
use std::rc::Rc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The third-party crates that the program's own crates (td-app, td-companion, td-core, td-host
/// and td-link) depend on directly, with the licence crates.io records for each. Slint offers a
/// choice of licences; Open TouchDeck uses it under GPL-3.0-only.
/// `components_match_the_manifests` keeps this list in step with the Cargo manifests.
pub const COMPONENTS: &[(&str, &str)] = &[
    ("aes-gcm", "Apache-2.0 OR MIT"),
    ("axum", "MIT"),
    ("base64", "MIT OR Apache-2.0"),
    ("dirs", "MIT OR Apache-2.0"),
    ("espflash", "MIT OR Apache-2.0"),
    ("flate2", "MIT OR Apache-2.0"),
    ("freedesktop-icons", "MIT"),
    ("fs2", "MIT/Apache-2.0"),
    ("futures-util", "MIT OR Apache-2.0"),
    ("getrandom", "MIT OR Apache-2.0"),
    ("gif", "MIT OR Apache-2.0"),
    ("hkdf", "MIT OR Apache-2.0"),
    ("hmac", "MIT OR Apache-2.0"),
    ("hyper-util", "MIT"),
    ("image", "MIT OR Apache-2.0"),
    ("inventory", "MIT OR Apache-2.0"),
    ("keyring", "MIT OR Apache-2.0"),
    ("libc", "MIT OR Apache-2.0"),
    ("mdns-sd", "Apache-2.0 OR MIT"),
    ("minisign-verify", "MIT"),
    ("num-bigint", "MIT OR Apache-2.0"),
    ("open", "MIT"),
    ("parking_lot", "MIT OR Apache-2.0"),
    ("reqwest", "MIT OR Apache-2.0"),
    ("resvg", "Apache-2.0 OR MIT"),
    ("roxmltree", "MIT OR Apache-2.0"),
    ("rustls", "Apache-2.0 OR ISC OR MIT"),
    ("rustls-platform-verifier", "MIT OR Apache-2.0"),
    ("semver", "MIT OR Apache-2.0"),
    ("serde", "MIT OR Apache-2.0"),
    ("serde_json", "MIT OR Apache-2.0"),
    ("serde_urlencoded", "MIT/Apache-2.0"),
    ("serialport", "MPL-2.0"),
    ("sha2", "MIT OR Apache-2.0"),
    ("slint", SLINT_LICENSE),
    ("srp", "MIT OR Apache-2.0"),
    ("subtle", "BSD-3-Clause"),
    ("tempfile", "MIT OR Apache-2.0"),
    ("thiserror", "MIT OR Apache-2.0"),
    ("time", "MIT OR Apache-2.0"),
    ("tokio", "MIT"),
    ("ts-rs", "MIT"),
    (
        "unicode-normalization",
        "(MIT OR Apache-2.0) AND Unicode-3.0",
    ),
    ("unicode-segmentation", "MIT OR Apache-2.0"),
    ("ureq", "MIT OR Apache-2.0"),
    ("url", "MIT OR Apache-2.0"),
    ("windows", "MIT OR Apache-2.0"),
    ("windows-core", "MIT OR Apache-2.0"),
    ("windows-future", "MIT OR Apache-2.0"),
    ("windows-sys", "MIT OR Apache-2.0"),
    ("x25519-dalek", "BSD-3-Clause"),
    ("zbus", "MIT"),
    ("zeroize", "Apache-2.0 OR MIT"),
    ("zip", "MIT"),
];
/// The licence Open TouchDeck takes Slint under.
const SLINT_LICENSE: &str = "GPL-3.0-only";

pub(crate) fn fill(ui: &AppWindow) {
    ui.set_version(VERSION.into());
    let components: Vec<_> = COMPONENTS
        .iter()
        .map(|(name, license)| Component {
            name: (*name).into(),
            license: (*license).into(),
        })
        .collect();
    ui.set_components(ModelRc::from(Rc::new(VecModel::from(components))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::collections::{BTreeMap, BTreeSet};

    const OWN: [&str; 6] = [
        "td-app",
        "td-companion",
        "td-core",
        "td-host",
        "td-link",
        "td-updater",
    ];

    fn metadata(args: &[&str]) -> Value {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let output = std::process::Command::new(cargo)
            .args(["metadata", "--format-version", "1", "--locked"])
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");
        assert!(
            output.status.success(),
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// Every third-party crate the program's crates declare as a normal, always-on dependency,
    /// on any platform. Dev, build and test-only optional dependencies don't ship.
    fn declared() -> BTreeSet<String> {
        let workspace = metadata(&["--no-deps"]);
        let mut names = BTreeSet::new();
        for package in workspace["packages"].as_array().unwrap() {
            if !OWN.contains(&package["name"].as_str().unwrap()) {
                continue;
            }
            for dependency in package["dependencies"].as_array().unwrap() {
                let name = dependency["name"].as_str().unwrap();
                if dependency["kind"].is_null()
                    && dependency["optional"] == false
                    && !OWN.contains(&name)
                {
                    names.insert(name.to_owned());
                }
            }
        }
        names
    }

    #[test]
    fn components_match_the_manifests() {
        let listed: BTreeSet<String> = COMPONENTS.iter().map(|(name, _)| (*name).into()).collect();
        let declared = declared();
        let missing: Vec<_> = declared.difference(&listed).collect();
        let stale: Vec<_> = listed.difference(&declared).collect();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "update COMPONENTS in about.rs: add {missing:?}, remove {stale:?}"
        );
        let mut sorted = COMPONENTS.to_vec();
        sorted.sort();
        assert_eq!(sorted, COMPONENTS, "COMPONENTS stays in alphabetical order");
    }

    #[test]
    fn components_show_the_licences_crates_io_records() {
        // Only this platform's crates resolve here; CI checks the others on their own runners.
        let resolved = metadata(&["--filter-platform", env!("TD_APP_TARGET")]);
        let mut licenses = BTreeMap::new();
        for package in resolved["packages"].as_array().unwrap() {
            let name = package["name"].as_str().unwrap();
            let license = package["license"].as_str().unwrap_or("").to_owned();
            licenses
                .entry(name.to_owned())
                .or_insert_with(BTreeSet::new)
                .insert(license);
        }
        let mut checked = 0;
        for (name, shown) in COMPONENTS {
            let Some(recorded) = licenses.get(*name) else {
                continue;
            };
            if *name == "slint" {
                assert!(
                    recorded
                        .iter()
                        .all(|l| l.split(" OR ").any(|choice| choice == SLINT_LICENSE))
                );
            } else {
                assert!(
                    recorded.contains(*shown),
                    "{name}: shown {shown:?}, crates.io {recorded:?}"
                );
            }
            checked += 1;
        }
        assert!(
            checked >= 40,
            "only {checked} components resolved on this platform"
        );
    }
}
