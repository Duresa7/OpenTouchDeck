fn main() {
    // Snapshot from PR #364 until its signing pipeline lands on main. Never
    // regenerate it. Once present, the signer's copy must remain byte-identical.
    let signer = "../../../.github/release-signer/updater.pub";
    println!("cargo:rerun-if-changed={signer}");
    println!("cargo:rerun-if-changed=updater.pub");
    match std::fs::read(signer) {
        Ok(key) => assert_eq!(
            key,
            std::fs::read("updater.pub").expect("committed updater key"),
            "updater and release signer public keys differ; never replace the durable key"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("cannot read release signer public key: {error}"),
    }
}
