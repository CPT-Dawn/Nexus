//! `nexus man`: the man pages, made from the same clap definitions as the
//! command line so they cannot drift. `nexus(1)` ends with the exit status
//! table; every command gets its own page (`nexus-wifi-join(1)`).

use std::io::Write;
use std::path::Path;

use clap::Command;
use clap_mangen::Man;

use crate::cli::EXIT_CODES;

/// Prints `nexus(1)` to stdout, or with `dir` writes the page of every
/// command into it.
pub fn write(root: &Command, dir: Option<&Path>) -> std::io::Result<()> {
    match dir {
        None => page(root, &mut std::io::stdout(), true),
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            let mut root = root.clone();
            root.build();
            all(&root, "nexus", dir, true)
        }
    }
}

fn all(command: &Command, name: &str, dir: &Path, top: bool) -> std::io::Result<()> {
    let mut file = std::fs::File::create(dir.join(format!("{name}.1")))?;
    let command = command.clone().name(name.to_owned()).bin_name(name.replace("-", " "));
    page(&command, &mut file, top)?;
    for sub in command.get_subcommands().filter(|c| !c.is_hide_set()) {
        let sub_name = format!("{name}-{}", sub.get_name());
        all(sub, &sub_name, dir, false)?;
    }
    Ok(())
}

fn page(command: &Command, out: &mut dyn Write, top: bool) -> std::io::Result<()> {
    let man = Man::new(command.clone());
    let mut buffer = Vec::new();
    man.render_title(&mut buffer)?;
    man.render_name_section(&mut buffer)?;
    man.render_synopsis_section(&mut buffer)?;
    man.render_description_section(&mut buffer)?;
    man.render_options_section(&mut buffer)?;
    man.render_subcommands_section(&mut buffer)?;
    if top {
        buffer.extend_from_slice(exit_status().as_bytes());
        buffer.extend_from_slice(see_also().as_bytes());
    }
    if command.get_version().is_some() {
        man.render_version_section(&mut buffer)?;
    }
    out.write_all(&buffer)
}

fn exit_status() -> String {
    let mut text = String::from(".SH EXIT STATUS\n");
    text.push_str(
        "Every command exits 0 on success. A refusal from the Daemon exits with a code \\
         of its own, so scripts can tell a mistyped password from a Network out of range.\n",
    );
    for (code, meaning) in EXIT_CODES {
        text.push_str(&format!(".TP\n\\fB{code}\\fR\n{}\n", roff(meaning)));
    }
    text
}

fn see_also() -> String {
    ".SH SEE ALSO\nThe wire protocol is documented in docs/ipc.md; \
     \\fBnexus completions\\fR prints shell completions.\n"
        .into()
}

fn roff(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('-', "\\-")
        .replace('`', "\\(ga")
}
