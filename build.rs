//! On Windows, builds the Restora icon and the file's details (name, publisher, version) into the exe. Windows shows
//! them in Explorer and the tray, and antivirus trusts a file that says what it is more than a bare one.
fn main() {
    println!("cargo:rerun-if-changed=assets/restora.ico");
    println!("cargo:rerun-if-changed=assets/restora.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/restora.ico")
        .set_manifest_file("assets/restora.manifest")
        .set("ProductName", "Restora Print")
        .set("FileDescription", "Restora Print: prints Restora orders on local printers")
        .set("CompanyName", "Restora")
        .set("LegalCopyright", "Copyright Restora")
        .set("OriginalFilename", "restora-pc-agent.exe")
        .set("InternalName", "restora-pc-agent");
    resource.compile().expect("the Windows resources");
}
