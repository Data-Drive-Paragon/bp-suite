use crate::octagon::Octagon;
use anyhow::{Result, Context};
use std::process::Command;

pub async fn launch_sqlit(octagon: &Octagon) -> Result<()> {
    log::info!("Checking if 'sqlit' CLI tool is installed...");
    
    let which_res = Command::new("which")
        .arg("sqlit")
        .output();
        
    let is_installed = which_res.is_ok() && which_res.unwrap().status.success();
    
    if !is_installed {
        anyhow::bail!(
            "'sqlit' CLI tool is not installed in your system.\n\n\
            Please install it using pip or pipx:\n\
              pip install sqlit-tui\n\
            or:\n\
              pipx install sqlit-tui\n\n\
            And then try running this command again!"
        );
    }
    
    log::info!("Registering database connection profiles in sqlit...");
    
    for config in &octagon.connections {
        let node_name = &config.name;
        let port_str = config.port.to_string();
        
        log::info!("  -> Registering connection profile for '{}' (port {})...", node_name, port_str);
        
        // Delete existing profile first to prevent duplicates or conflicts
        let _ = Command::new("sqlit")
            .args(&["connections", "delete", node_name])
            .output();
            
        // Add new connection profile
        let status = Command::new("sqlit")
            .args(&[
                "connections", "add", "postgresql",
                "--name", node_name,
                "--server", "localhost",
                "--port", &port_str,
                "--database", &config.dbname,
                "--username", &config.user,
                "--password", &config.pass,
            ])
            .status()
            .context(format!("Failed to register connection profile for '{}'", node_name))?;
            
        if !status.success() {
            log::warn!("  -> Failed to register connection profile for '{}'", node_name);
        }
    }
    
    // Check if we are running inside KDE's Konsole terminal emulator
    let use_konsole = std::env::var("KONSOLE_VERSION").is_ok() 
        || std::env::var("TERM_PROGRAM").as_ref().map(|s| s.as_str()) == Ok("konsole");
        
    let konsole_exists = if use_konsole {
        Command::new("which")
            .arg("konsole")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        false
    };
    
    if use_konsole && konsole_exists {
        log::info!("Detected KDE Konsole terminal environment. Spawning sqlit in a dedicated sleek window...");
        
        let mut child = Command::new("konsole")
            .args(&[
                "--separate",
                "--notransparency",
                "--hide-menubar",
                "--hide-toolbars",
                "--hide-tabbar",
                "-e",
                "sqlit",
            ])
            .spawn()
            .context("Failed to spawn separate KDE Konsole window for sqlit")?;
            
        let _ = child.wait().context("KDE Konsole process encountered an error during execution")?;
    } else {
        log::info!("Opening sqlit TUI directly in the current window...");
        
        let mut child = Command::new("sqlit")
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .context("Failed to spawn sqlit TUI process")?;
            
        let _ = child.wait().context("sqlit TUI process encountered an error during execution")?;
    }
    
    Ok(())
}
