use std::path::PathBuf;

use crate::config::ConfinuumConfig;
use anyhow::{anyhow, Result};
use crossterm::style::{Color, Stylize};

#[derive(Debug)]
struct MockDirEntry {
    name: String,
    entries: Vec<MockDirEntry>,
}

impl MockDirEntry {
    fn new_dir(name: String, entries: Vec<MockDirEntry>) -> Self {
        Self { name, entries }
    }

    fn dir_add_entry(&mut self, entry: MockDirEntry) {
        self.entries.push(entry);
    }

    fn dir_find_entry_mut(&mut self, name: &str) -> Option<&mut MockDirEntry> {
        for entry in &mut self.entries {
            if entry.name == name {
                return Some(entry);
            }
        }
        None
    }

    fn build_tree(&mut self, path: &PathBuf, depth: usize) {
        if depth < path.components().count() {
            let item = &path.components().nth(depth).unwrap();

            let dir = match self.dir_find_entry_mut(item.as_os_str().to_str().unwrap()) {
                Some(dir) => dir,
                None => {
                    let dir = MockDirEntry::new_dir(
                        item.as_os_str().to_str().unwrap().to_string(),
                        Vec::new(),
                    );
                    self.dir_add_entry(dir);
                    self.dir_find_entry_mut(item.as_os_str().to_str().unwrap())
                        .unwrap()
                }
            };
            dir.build_tree(path, depth + 1)
        }
    }

    fn print_tree(&self, depth: usize, last: bool) {
        let (color, icon) = if self.entries.is_empty() {
            (Color::Reset, " \u{1F5CB}")
        } else {
            (Color::Blue, " \u{1F5C1} ")
        };
        if depth == 0 {
            println!("{}", self.name.clone().yellow());
        } else {
            let indent = (((depth as usize) - 1) * 4).checked_sub(1).unwrap_or(0);
            println!(
                "{}{:indent$}{}{} {}",
                if indent == 0 { "" } else { "│" },
                "",
                if last { "└──" } else { "├──" },
                icon,
                self.name.clone().with(color),
                // Test
            );
        }
        for (idx, entry) in self.entries.iter().enumerate() {
            entry.print_tree(depth + 1, idx == self.entries.len() - 1);
        }
    }
}

pub fn show(name: String) -> Result<()> {
    let config = ConfinuumConfig::load()?;
    let entry = config
        .entries
        .get(&name)
        .ok_or_else(|| anyhow!("No entry named {} found", name))?;

    let mut root = MockDirEntry::new_dir(
        format!(
            "{} in {}",
            &name,
            entry.target_dir.as_ref().unwrap().to_string_lossy()
        ),
        Vec::new(),
    );
    for file in &entry.files {
        root.build_tree(file, 0);
    }
    root.print_tree(0, false);

    /* let mut stdout = std::io::stdout();
    queue!(
        stdout,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(format!(
            "{}: {} files in {}\n",
            name.bold().yellow(),
            entry.files.len(),
            entry.target_dir.as_ref().unwrap().display()
        )),
    )?;

    for file in &entry.files {
        queue!(stdout, Print(format!("- {}\n", file.display())))?;
    }

    stdout.flush()?; */
    Ok(())
}
