pub async fn browse_file_impl() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().to_string());
    Ok(path)
}

pub async fn browse_attachment_file_impl() -> Result<Option<String>, String> {
    browse_file_impl().await
}
