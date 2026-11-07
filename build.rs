fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app-icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("Failed to compile windows resource: {e}");
            std::process::exit(1);
        }
    }
}
