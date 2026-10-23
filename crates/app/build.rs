// Значок и сведения о версии в nebula-client.exe (для ярлыков, проводника и «Программ и компонентов»).
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("../../dist/windows/nebula-client.ico")
                .set("ProductName", "Nebula Client")
                .set("FileDescription", "Nebula Client")
                .set("LegalCopyright", "GPL-3.0");
            if let Err(e) = res.compile() {
                println!("cargo:warning=no icon resource: {e}");
            }
        }
    }
    println!("cargo:rerun-if-changed=../../dist/windows/nebula-client.ico");
}
