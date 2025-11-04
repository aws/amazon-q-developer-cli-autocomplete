use std::path::Path;

use crate::Error;

/// Create legacy q wrapper script for backward compatibility
pub async fn create_q_wrapper(install_dir: &Path) -> Result<(), Error> {
    let wrapper_path = install_dir.join("q");

    // Check what exists and handle appropriately
    if wrapper_path.exists() {
        let metadata = tokio::fs::symlink_metadata(&wrapper_path).await?;

        if metadata.is_symlink() {
            // It's a symlink (likely from old installation) - safe to replace
            tokio::fs::remove_file(&wrapper_path).await?;
        } else if is_our_wrapper(&wrapper_path).await? {
            // It's our wrapper from previous install - safe to replace
            tokio::fs::remove_file(&wrapper_path).await?;
        } else {
            // It's something else (assume old Q CLI) - safe to replace per our assumption
            tokio::fs::remove_file(&wrapper_path).await?;
        }
    }

    // Create wrapper script content
    let wrapper_content = format!(
        "#!/bin/sh\n\"{}/kiro-cli\" --show-legacy-warning \"$@\"\n",
        install_dir.display()
    );

    // Write wrapper script
    tokio::fs::write(&wrapper_path, wrapper_content).await?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&wrapper_path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&wrapper_path, perms).await?;
    }

    Ok(())
}

/// Create legacy qchat wrapper script for backward compatibility
pub async fn create_qchat_wrapper(install_dir: &Path) -> Result<(), Error> {
    let wrapper_path = install_dir.join("qchat");

    // Remove existing qchat command if it exists
    if wrapper_path.exists() {
        tokio::fs::remove_file(&wrapper_path).await?;
    }

    // Create wrapper script content that calls q chat
    let wrapper_content = format!("#!/bin/sh\n\"{}/q\" chat \"$@\"\n", install_dir.display());

    // Write wrapper script
    tokio::fs::write(&wrapper_path, wrapper_content).await?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&wrapper_path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&wrapper_path, perms).await?;
    }

    Ok(())
}

/// Check if the existing q command is our wrapper script
async fn is_our_wrapper(path: &Path) -> Result<bool, Error> {
    if let Ok(content) = tokio::fs::read_to_string(path).await {
        // Check if it contains our signature
        Ok(content.contains("--show-legacy-warning") && content.contains("kiro-cli"))
    } else {
        Ok(false)
    }
}
