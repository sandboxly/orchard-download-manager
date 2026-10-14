//! File categories (by extension): the list's type filter and, if sorting
//! is on, the folder a file goes to.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Category {
    Video,
    Music,
    Documents,
    Archives,
    Programs,
    Images,
    Other,
}

impl Category {
    pub const ALL: [Category; 7] =
        [Category::Video, Category::Music, Category::Documents, Category::Archives, Category::Programs, Category::Images, Category::Other];

    /// Also the folder name when sorting by category.
    pub fn label(self) -> &'static str {
        match self {
            Category::Video => "Video",
            Category::Music => "Music",
            Category::Documents => "Documents",
            Category::Archives => "Archives",
            Category::Programs => "Programs",
            Category::Images => "Images",
            Category::Other => "Other",
        }
    }

    pub fn of(name: &str) -> Category {
        let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        Category::of_ext(&ext)
    }

    pub fn of_ext(ext: &str) -> Category {
        match ext {
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp" | "ts" | "m2ts" | "vob" | "ogv" => {
                Category::Video
            }
            "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" | "aiff" | "aif" | "alac" | "ape" | "mka" => Category::Music,
            "pdf" | "epub" | "mobi" | "djvu" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "xls" | "xlsx" | "ods" | "csv" | "ppt"
            | "pptx" | "odp" | "pages" | "numbers" | "key" => Category::Documents,
            "zip" | "rar" | "7z" | "gz" | "bz2" | "xz" | "zst" | "tar" | "tgz" | "tbz2" | "txz" | "lz" | "lzma" | "cab" | "iso" | "img" => {
                Category::Archives
            }
            "exe" | "msi" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" | "apk" | "app" | "msix" | "jar" | "run" | "sh" | "bin" => Category::Programs,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "heic" | "avif" | "raw" | "psd" => Category::Images,
            _ => Category::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_extension() {
        assert_eq!(Category::of("Movie.MKV"), Category::Video);
        assert_eq!(Category::of("setup.exe"), Category::Programs);
        assert_eq!(Category::of("linux.iso"), Category::Archives);
        assert_eq!(Category::of("README"), Category::Other);
    }
}
