// SPDX-License-Identifier: MPL-2.0

use std::{fs::File, io::Read, path::Path};

pub const MARKER: &str = "yaagl-squircle wrapper v1";

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The sh script that replaces the Wine loader and execs the renamed original.
pub fn render(loader: &str, bridge: &Path, icon: Option<&Path>) -> String {
    let mut s = format!(
        "#!/bin/sh\n# {MARKER} - managed file, remove with: yaagl-squircle revert\n\
         dir=$(dirname \"$0\")\n\
         export DYLD_INSERT_LIBRARIES={}${{DYLD_INSERT_LIBRARIES:+:$DYLD_INSERT_LIBRARIES}}\n",
        sh_quote(&bridge.to_string_lossy())
    );
    if let Some(icon) = icon {
        s.push_str(&format!(
            "export YAAGL_SQUIRCLE_ICON={}\n",
            sh_quote(&icon.to_string_lossy())
        ));
    }
    s.push_str(&format!("exec \"$dir/{loader}.real\" \"$@\"\n"));
    s
}

/// True if the file starts with our marker. Reads only the head, since the original
/// loader is a large Mach-O binary.
pub fn is_wrapper(path: &Path) -> bool {
    let mut head = [0u8; 512];
    let Ok(n) = File::open(path).and_then(|mut f| f.read(&mut head)) else {
        return false;
    };
    head[..n]
        .windows(MARKER.len())
        .any(|w| w == MARKER.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_spaces_and_single_quotes() {
        let s = render("wine64", Path::new("/a b/it's.dylib"), None);
        assert!(s.contains(r"'/a b/it'\''s.dylib'"));
        assert!(s.contains("wine64.real"));
        assert!(!s.contains("YAAGL_SQUIRCLE_ICON"));
    }

    #[test]
    fn icon_line_only_with_icon() {
        let s = render("wine", Path::new("/b.dylib"), Some(Path::new("/i.png")));
        assert!(s.contains("export YAAGL_SQUIRCLE_ICON='/i.png'"));
    }
}
