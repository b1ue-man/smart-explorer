use super::prelude::*;
use super::*;

#[cfg(test)]
pub(in crate::app) mod omni_tests {
    use super::*;

    #[test]
    fn classify_modes() {
        assert_eq!(omni_mode("report"), OmniMode::Filter);
        assert_eq!(omni_mode(""), OmniMode::Filter);
        assert_eq!(omni_mode(">new"), OmniMode::Command);
        assert_eq!(omni_mode("  > refresh"), OmniMode::Command);
        assert_eq!(omni_mode(".."), OmniMode::Path);
        assert_eq!(omni_mode("../.."), OmniMode::Path);
        assert_eq!(omni_mode("C:\\Users"), OmniMode::Path);
        assert_eq!(omni_mode("C:"), OmniMode::Path);
        assert_eq!(omni_mode("~"), OmniMode::Path);
        assert_eq!(omni_mode("\\\\server\\share"), OmniMode::Path);
        assert_eq!(omni_mode("a/b"), OmniMode::Path);
        // A leading slash is the explicit folder-search trigger (not a path).
        assert_eq!(omni_mode("/proj"), OmniMode::FolderSearch);
        assert_eq!(omni_mode("/"), OmniMode::FolderSearch);
        assert_eq!(omni_mode("  /docs"), OmniMode::FolderSearch);
        // Plain names (even with dots) stay filters.
        assert_eq!(omni_mode("file.txt"), OmniMode::Filter);
        assert_eq!(omni_mode("v1.2.3"), OmniMode::Filter);
    }

    #[test]
    fn up_levels() {
        assert_eq!(omni_up_levels(".."), Some(1));
        assert_eq!(omni_up_levels("..."), Some(2));
        assert_eq!(omni_up_levels("...."), Some(3));
        assert_eq!(omni_up_levels("../.."), Some(2));
        assert_eq!(omni_up_levels("..\\..\\.."), Some(3));
        assert_eq!(omni_up_levels("../foo"), None);
        assert_eq!(omni_up_levels("foo"), None);
        assert_eq!(omni_up_levels("."), None);
    }

    #[test]
    fn fuzzy() {
        assert!(fuzzy_contains("Neuer Ordner", "no"));
        assert!(fuzzy_contains("Neuer Ordner", "ordner"));
        assert!(fuzzy_contains("Aktualisieren", ""));
        assert!(fuzzy_contains("Terminal hier öffnen", "term"));
        assert!(!fuzzy_contains("Neuer Ordner", "xyz"));
        assert!(!fuzzy_contains("abc", "abcd"));
    }

    #[test]
    fn treemap_areas_proportional() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let w = [50.0_f64, 30.0, 20.0];
        let rects = treemap_layout(&w, rect);
        let total_area: f32 = rects.iter().map(|r| r.area()).sum();
        assert!((total_area - rect.area()).abs() < rect.area() * 0.01);
        // Each cell stays within bounds and is proportional.
        let big = rects[0].area();
        let small = rects[2].area();
        assert!(big > small);
        for r in &rects {
            assert!(rect.contains_rect(r.shrink(0.5)));
        }
    }

    #[test]
    fn path_expansion() {
        let home = std::path::Path::new("/home/u");
        assert_eq!(expand_omni_path("~", home, ""), "/home/u");
        // bare drive completes to a root
        assert_eq!(expand_omni_path("C:", home, ""), format!("C:{}", std::path::MAIN_SEPARATOR));
        // ~/sub expands under home
        let got = expand_omni_path("~/docs", home, "");
        assert!(got.ends_with("docs"));
        assert!(got.starts_with("/home/u"));
    }

    #[test]
    fn temp_names_are_sanitized() {
        assert_eq!(safe_temp_name("a/b\\c:d.txt"), "a_b_c_d.txt");
        assert_eq!(safe_temp_name("   "), "datei");
    }

    #[test]
    fn download_part_is_sibling_not_final() {
        let dest = PathBuf::from("C:/tmp/report.txt");
        let part = download_part_path(&dest);
        assert_eq!(part.parent(), dest.parent());
        assert_ne!(part.file_name(), dest.file_name());
        assert!(part
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("smart-explorer.part"));
    }

    #[test]
    fn remote_temp_path_stays_sibling() {
        let tmp = remote_temp_path("/remote/dir/report.txt");
        assert!(tmp.starts_with("/remote/dir/report.txt.se-upload-"));
        assert!(tmp.ends_with(".part"));
    }
}
