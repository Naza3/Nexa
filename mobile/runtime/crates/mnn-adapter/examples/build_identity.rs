//! Fixed key=value output for build evidence; no model paths or user content.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let identity = mnn_adapter::build_identity()?;
    println!("identity_schema=nexa-mnn-native-build-v1");
    println!(
        "artifact_manifest_sha256={}",
        identity.artifact_manifest_sha256
    );
    println!("upstream_commit={}", identity.upstream_commit);
    println!("patch_sha256={}", identity.patch_sha256);
    println!("policy_sha256={}", identity.policy_sha256);
    println!("target={}", identity.target);
    println!("compiler={}", identity.compiler);
    Ok(())
}
