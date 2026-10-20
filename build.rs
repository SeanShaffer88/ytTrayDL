fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "ytTrayDL");
        res.set("FileDescription", "ytTrayDL — tray companion for yt-dlp");
        res.set("CompanyName", "Sean Shaffer");
        if let Err(e) = res.compile() {
            println!("cargo:warning=winresource failed: {e}");
        }
    }
}
