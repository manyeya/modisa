// `modisa logos [status|install|uninstall]`: agents' logos in the terminal (see src/platform/logos.rs). The TUI installs
// them the first time it starts; this shows where they stand, puts them back, or takes them out for good.
use crate::platform::logos::{install_logos, logo_status, logos_visible, uninstall_logos};

pub async fn run_logos(verb: Option<&str>) -> i32 {
    match run(verb.unwrap_or("status")).await {
        Ok(code) => code,
        Err(e) => {
            errln!("modisa: {e}");
            1
        }
    }
}

async fn run(verb: &str) -> Result<i32, String> {
    if verb == "install" {
        let done = install_logos().await?;
        outln!("installed modisa's logo font{}", if done.is_empty() { String::new() } else { format!(" and set up {}", done.join(", ")) });
        outln!("restart your terminal (in VS Code, reload the window) to see the logos");
        return Ok(0);
    }
    if verb == "uninstall" {
        uninstall_logos()?;
        outln!("removed modisa's logo font and its terminal settings; the sidebar shows plain marks, and the TUI won't install them again");
        return Ok(0);
    }
    if verb != "status" {
        errln!("usage: modisa logos [status|install|uninstall]");
        return Ok(2);
    }
    let s = logo_status()?;
    match &s.font {
        Some(font) => outln!("font: {font}{}", if s.current { "" } else { " (an older one: modisa logos install updates it)" }),
        None => outln!("font: not installed (modisa logos install)"),
    }
    for t in &s.terminals {
        outln!("{:<15} {}  {}", t.name, if t.configured { "set up" } else { "not set up" }, t.path);
    }
    outln!("this terminal: {}", if logos_visible() { "shows the logos" } else { "shows plain marks" });
    Ok(0)
}
