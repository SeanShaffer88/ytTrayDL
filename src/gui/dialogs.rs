//! Native file and folder pickers.

pub fn pick_file(filter_name: &str, ext: &str) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new()
            .add_filter(filter_name, &[ext])
            .pick_file()
    }
    #[cfg(not(windows))]
    {
        let _ = (filter_name, ext);
        None
    }
}

pub fn pick_save_json(default_name: &str) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new()
            .set_file_name(default_name)
            .add_filter("JSON settings", &["json"])
            .save_file()
    }
    #[cfg(not(windows))]
    {
        let _ = default_name;
        None
    }
}

pub fn pick_folder(initial: &std::path::Path) -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        rfd::FileDialog::new().set_directory(initial).pick_folder()
    }
    #[cfg(not(windows))]
    {
        let _ = initial;
        None
    }
}
