//! OpenUSD sublayer stack file operations (create / reorder / delete / import).

use std::fs;
use std::path::{Path, PathBuf};

use crate::layer_stack::{
    imported_portfolio_layer_filename, portfolio_import_insert_index, session_layer_usda,
    signals_layer_usda, sp500_universe_layer_usda, workstation_root_layer_header,
    SESSION_LAYER_FILENAME, SIGNALS_LAYER_FILENAME, SP500_UNIVERSE_LAYER_FILENAME,
};

const EMPTY_LAYER_SCAFFOLD: &str = "#usda 1.0\n(\n)\n\ndef Scope \"MarketLab\"\n{\n}\n";

/// Normalize `@./session.usda@` → `session.usda`.
pub fn sublayer_ref_to_filename(reference: &str) -> Option<String> {
    let trimmed = reference.trim();
    let inner = trimmed.strip_prefix('@')?.strip_suffix('@')?;
    let leaf = Path::new(inner)
        .file_name()
        .and_then(|name| name.to_str())?;
    Some(leaf.to_string())
}

/// Extract ordered sublayer filenames from root layer USDA text.
pub fn parse_ordered_sublayer_filenames(root_usda: &str) -> Vec<String> {
    let Some(block) = extract_sublayers_block(root_usda) else {
        return Vec::new();
    };
    block
        .lines()
        .filter_map(|line| {
            let token = line.trim().trim_end_matches(',').trim();
            if token.is_empty() {
                return None;
            }
            sublayer_ref_to_filename(token)
        })
        .collect()
}

/// Write ordered sublayer references back into the root layer on disk.
pub fn write_ordered_sublayers(root_layer_path: &Path, filenames: &[String]) -> Result<(), String> {
    let text = fs::read_to_string(root_layer_path)
        .map_err(|err| format!("Failed to read root layer: {err}"))?;
    let refs: Vec<String> = filenames
        .iter()
        .map(|name| format!("@./{name}@"))
        .collect();
    let updated = replace_sublayers_block(&text, &refs);
    fs::write(root_layer_path, updated).map_err(|err| format!("Disk I/O write error: {err}"))
}

/// INSERT: add an existing on-disk sublayer filename at a specific stack index.
pub fn insert_sublayer_at(
    root_layer_path: &Path,
    index: usize,
    filename: &str,
) -> Result<(), String> {
    let filename = normalize_layer_filename(filename);
    let text = fs::read_to_string(root_layer_path)
        .map_err(|err| format!("Failed to read root layer: {err}"))?;
    let mut ordered = parse_ordered_sublayer_filenames(&text);
    if ordered.iter().any(|layer| layer == &filename) {
        return Ok(());
    }
    let index = index.min(ordered.len());
    ordered.insert(index, filename);
    write_ordered_sublayers(root_layer_path, &ordered)
}

/// PREPEND: insert at the strongest (front) composition slot.
pub fn prepend_sublayer(root_layer_path: &Path, filename: &str) -> Result<(), String> {
    insert_sublayer_at(root_layer_path, 0, filename)
}

/// Copy an external portfolio `.usda` into the project directory and register it as a sublayer.
pub fn import_external_portfolio_layer(
    root_layer_path: &Path,
    source_usda_path: &Path,
) -> Result<String, String> {
    ensure_workstation_sublayer_stack(root_layer_path)?;

    if !source_usda_path.is_file() {
        return Err(format!(
            "Portfolio import source not found: {}",
            source_usda_path.display()
        ));
    }

    let source_stem = source_usda_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("portfolio");
    let parent = root_layer_path
        .parent()
        .ok_or_else(|| "Root layer has no parent directory.".to_string())?;

    let mut filename = imported_portfolio_layer_filename(source_stem);
    let mut target = parent.join(&filename);
    let mut suffix = 1u32;
    while target.exists() {
        filename = format!(
            "{}_{suffix}.usda",
            imported_portfolio_layer_filename(source_stem).trim_end_matches(".usda")
        );
        target = parent.join(&filename);
        suffix += 1;
    }

    fs::copy(source_usda_path, &target)
        .map_err(|err| format!("Failed to copy portfolio layer: {err}"))?;

    let text = fs::read_to_string(root_layer_path)
        .map_err(|err| format!("Failed to read root layer: {err}"))?;
    let ordered = parse_ordered_sublayer_filenames(&text);
    let insert_at = portfolio_import_insert_index(&ordered);
    insert_sublayer_at(root_layer_path, insert_at, &filename)?;
    Ok(filename)
}

/// Promote an inline root document into the standard workstation sublayer stack on disk.
pub fn ensure_workstation_sublayer_stack(root_layer_path: &Path) -> Result<(), String> {
    let text = fs::read_to_string(root_layer_path)
        .map_err(|err| format!("Failed to read root layer: {err}"))?;
    let ordered = parse_ordered_sublayer_filenames(&text);
    if !ordered.is_empty() {
        return Ok(());
    }

    let parent = root_layer_path
        .parent()
        .ok_or_else(|| "Root layer has no parent directory.".to_string())?;
    let session_path = parent.join(SESSION_LAYER_FILENAME);
    if !session_path.exists() {
        let session_body = if text.contains("def Scope \"MarketLab\"") {
            text
        } else {
            session_layer_usda()
        };
        fs::write(&session_path, session_body)
            .map_err(|err| format!("Disk I/O write error: {err}"))?;
    }
    let signals_path = parent.join(SIGNALS_LAYER_FILENAME);
    if !signals_path.exists() {
        fs::write(signals_path, signals_layer_usda())
            .map_err(|err| format!("Disk I/O write error: {err}"))?;
    }
    let universe_path = parent.join(SP500_UNIVERSE_LAYER_FILENAME);
    if !universe_path.exists() {
        fs::write(&universe_path, sp500_universe_layer_usda())
            .map_err(|err| format!("Disk I/O write error: {err}"))?;
    }
    fs::write(root_layer_path, workstation_root_layer_header())
        .map_err(|err| format!("Disk I/O write error: {err}"))?;
    Ok(())
}

/// CREATE: scaffold a new sublayer beside the root document and append it to the stack.
pub fn create_and_insert_sublayer(root_layer_path: &Path, filename: &str) -> Result<(), String> {
    let filename = normalize_layer_filename(filename);
    let layer_path = layer_path_for_root(root_layer_path, &filename)?;
    if layer_path.exists() {
        return Err(format!("Sublayer file already exists: {filename}"));
    }

    fs::write(&layer_path, EMPTY_LAYER_SCAFFOLD)
        .map_err(|err| format!("Disk I/O write error: {err}"))?;

    let mut ordered = parse_ordered_sublayer_filenames(
        &fs::read_to_string(root_layer_path)
            .map_err(|err| format!("Failed to read root layer: {err}"))?,
    );
    if !ordered.iter().any(|layer| layer == &filename) {
        ordered.push(filename);
        write_ordered_sublayers(root_layer_path, &ordered)?;
    }
    Ok(())
}

/// UPDATE: reorder sublayer composition priority on the root metadata block.
pub fn reorder_sublayer(
    root_layer_path: &Path,
    from_index: usize,
    to_index: usize,
) -> Result<(), String> {
    let text = fs::read_to_string(root_layer_path)
        .map_err(|err| format!("Failed to read root layer: {err}"))?;
    let mut paths = parse_ordered_sublayer_filenames(&text);
    if from_index >= paths.len() || to_index >= paths.len() {
        return Err("Layer reorder out-of-bounds.".to_string());
    }
    let target = paths.remove(from_index);
    paths.insert(to_index, target);
    write_ordered_sublayers(root_layer_path, &paths)
}

/// DELETE: detach a sublayer reference and remove its on-disk file.
pub fn remove_sublayer(root_layer_path: &Path, filename: &str) -> Result<(), String> {
    let filename = normalize_layer_filename(filename);
    let text = fs::read_to_string(root_layer_path)
        .map_err(|err| format!("Failed to read root layer: {err}"))?;
    let mut paths = parse_ordered_sublayer_filenames(&text);
    let before = paths.len();
    paths.retain(|layer| layer != &filename);
    if paths.len() == before {
        return Err(format!("Sublayer not found in stack: {filename}"));
    }
    write_ordered_sublayers(root_layer_path, &paths)?;

    let layer_path = layer_path_for_root(root_layer_path, &filename)?;
    if layer_path.exists() {
        fs::remove_file(layer_path).map_err(|err| format!("Failed to delete sublayer: {err}"))?;
    }
    Ok(())
}

pub fn layer_path_for_root(root_layer_path: &Path, filename: &str) -> Result<PathBuf, String> {
    let parent = root_layer_path
        .parent()
        .ok_or_else(|| "Root layer has no parent directory.".to_string())?;
    Ok(parent.join(normalize_layer_filename(filename)))
}

fn normalize_layer_filename(filename: &str) -> String {
    let leaf = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(filename);
    if leaf.ends_with(".usda") || leaf.ends_with(".usd") {
        leaf.to_string()
    } else {
        format!("{leaf}.usda")
    }
}

fn extract_sublayers_block(text: &str) -> Option<&str> {
    let marker = "subLayers";
    let start = text.find(marker)?;
    let after = &text[start..];
    let open = after.find('[')? + start;
    let slice = &text[open..];
    let close = slice.find(']')?;
    Some(&slice[..=close])
}

fn replace_sublayers_block(text: &str, refs: &[String]) -> String {
    let Some(block) = extract_sublayers_block(text) else {
        return insert_sublayers_block(text, refs);
    };
    let replacement = render_sublayers_block(refs);
    text.replacen(block, &replacement, 1)
}

fn insert_sublayers_block(text: &str, refs: &[String]) -> String {
    if let Some(close_paren) = text.find("\n)\n") {
        let mut out = String::new();
        out.push_str(&text[..close_paren]);
        out.push_str("\n");
        out.push_str(&render_sublayers_block(refs));
        out.push_str(&text[close_paren..]);
        return out;
    }
    format!("{text}\n{}", render_sublayers_block(refs))
}

fn render_sublayers_block(refs: &[String]) -> String {
    let mut out = String::from("    subLayers = [\n");
    for reference in refs {
        out.push_str("        ");
        out.push_str(reference);
        out.push_str("\n");
    }
    out.push_str("    ]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn inserts_imported_layer_before_universe_base() {
        let dir = std::env::temp_dir().join(format!("ml_import_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let root = dir.join("project.usda");
        fs::write(&root, workstation_root_layer_header()).expect("root");
        fs::write(dir.join(SESSION_LAYER_FILENAME), session_layer_usda()).expect("session");
        fs::write(dir.join(SIGNALS_LAYER_FILENAME), signals_layer_usda()).expect("signals");
        fs::write(
            dir.join(SP500_UNIVERSE_LAYER_FILENAME),
            sp500_universe_layer_usda(),
        )
        .expect("universe");

        let source = dir.join("Alpha_Portfolio.usda");
        fs::write(
            &source,
            r#"#usda 1.0
(
)
def Scope "MarketLab"
{
    def Scope "Portfolios" { }
}
"#,
        )
        .expect("source");

        let imported =
            import_external_portfolio_layer(&root, &source).expect("import");
        assert!(imported.starts_with("imported_Alpha_Portfolio"));
        let ordered = parse_ordered_sublayer_filenames(&fs::read_to_string(&root).expect("read"));
        assert_eq!(
            ordered,
            vec![
                SESSION_LAYER_FILENAME.to_string(),
                SIGNALS_LAYER_FILENAME.to_string(),
                imported,
                SP500_UNIVERSE_LAYER_FILENAME.to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_and_rewrites_sublayer_order() {
        let dir = std::env::temp_dir().join(format!("ml_sublayer_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let root = dir.join("root.usda");
        fs::write(
            &root,
            "#usda 1.0\n(\n    subLayers = [\n        @./a.usda@\n        @./b.usda@\n    ]\n)\n",
        )
        .expect("write");

        reorder_sublayer(&root, 0, 1).expect("reorder");
        let ordered = parse_ordered_sublayer_filenames(&fs::read_to_string(&root).expect("read"));
        assert_eq!(ordered, vec!["b.usda", "a.usda"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
