//! Point hermes' default model at a running profile.
//!
//! The file holds secrets, so only two lines of the `model:` block are edited and
//! nothing is ever printed.

use crate::{Result, paths};

/// Rewrite `default:` and `context_length:` inside the top-level `model:` block.
pub fn rewrite(yaml: &str, model: &str, context: u32) -> String {
    let mut inside = false;
    yaml.split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches('\n');
            let starts_block = !inside && body.starts_with("model:");
            let ends_block = inside && !body.is_empty() && !body.starts_with(' ');
            if starts_block {
                inside = true;
            }
            let replaced = match inside && !ends_block {
                true if body.starts_with("  default:") => Some(format!("  default: {model}")),
                true if body.starts_with("  context_length:") => {
                    Some(format!("  context_length: {context}"))
                }
                _ => None,
            };
            if ends_block {
                inside = false;
            }
            match replaced {
                Some(new) if line.ends_with('\n') => new + "\n",
                Some(new) => new,
                None => line.to_string(),
            }
        })
        .collect()
}

pub fn update(model: &str, context: u32) -> Result<()> {
    let path = paths::hermes_config();
    let old = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    paths::write_file(&path, &rewrite(&old, model, context))
}
