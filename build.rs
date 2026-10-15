//! On Windows, MyPDF.exe carries its icon and version details (product name,
//! version, copyright), which Explorer shows and code signing checks.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/windows/MyPDF.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("packaging/windows/MyPDF.ico")
            .set("ProductName", "MyPDF")
            .set("FileDescription", "MyPDF")
            .set("CompanyName", "MyPDF contributors")
            .set(
                "LegalCopyright",
                "© 2026 MyPDF contributors · MIT OR Apache-2.0",
            );
        resource
            .compile()
            .expect("Windows icon and version details could not be added");
    }
}
