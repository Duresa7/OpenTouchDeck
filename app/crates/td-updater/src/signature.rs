use std::{fs::File, io::Read, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use semver::Version;

use crate::Failure;

fn unwrap(text: &[u8]) -> Result<String, Failure> {
    let text = std::str::from_utf8(text).map_err(|_| Failure::Signature)?;
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|_| Failure::Signature)?;
    String::from_utf8(bytes).map_err(|_| Failure::Signature)
}

pub(crate) fn verify(
    path: &Path,
    wrapped: &[u8],
    public_key: &str,
    filename: &str,
    version: &Version,
) -> Result<(), Failure> {
    let key = PublicKey::decode(&unwrap(public_key.as_bytes())?).map_err(|_| Failure::Signature)?;
    let text = unwrap(wrapped)?;
    let signature = Signature::decode(&text).map_err(|_| Failure::Signature)?;
    // Streaming verification requires ED (BLAKE2b-512 prehash) and also verifies
    // the global signature over the payload signature and trusted comment.
    let mut verifier = key
        .verify_stream(&signature)
        .map_err(|_| Failure::Signature)?;
    let mut file = File::open(path)?;
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count]);
    }
    verifier.finalize().map_err(|_| Failure::Signature)?;
    // Require the signer's exact comment shape; authenticated metadata, not the
    // unsigned GitHub tag, authorizes this version and installer name.
    let comment = text.lines().nth(2).ok_or(Failure::Signature)?;
    let fields: Vec<_> = comment.split('\t').collect();
    if fields.len() != 3
        || fields[0]
            .strip_prefix("trusted comment: timestamp:")
            .and_then(|timestamp| timestamp.parse::<u64>().ok())
            .is_none()
        || fields[1] != format!("file:{filename}")
        || fields[2] != format!("version:{version}")
    {
        return Err(Failure::Signature);
    }
    Ok(())
}
