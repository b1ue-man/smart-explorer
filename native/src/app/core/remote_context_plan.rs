//! Which entries the remote context menu offers, decided before rendering so
//! the supported operations are explicit and testable. Copy and paste exist
//! on every platform (the app keeps its own clipboard); "Für andere Programme
//! bereitstellen" needs an OS file clipboard other programs read.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextEntryKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteRowSelection {
    ClickedOnly,
    MultipleIncludingClicked,
    ClickedOutsideSelection,
}

impl RemoteRowSelection {
    pub(in crate::app) fn includes_clicked(self) -> bool {
        !matches!(self, Self::ClickedOutsideSelection)
    }

    pub(in crate::app) fn is_single(self) -> bool {
        matches!(self, Self::ClickedOnly)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextSubject {
    Row {
        entry_kind: RemoteContextEntryKind,
        selection: RemoteRowSelection,
    },
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct RemoteContextCapabilities {
    pub(in crate::app) open_with_chooser: bool,
    /// The platform has a file clipboard other programs read (Windows).
    pub(in crate::app) file_clipboard: bool,
}

/// Menu availability is calculated before rendering, making the supported
/// remote operations explicit and deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextAction {
    Open,
    OpenWith,
    DownloadTo,
    CopyToClipboard,
    ProvideForPrograms,
    Rename,
    Delete,
    ToggleFavorite,
    CopyPath,
    AnalyzeDirectory,
    Paste,
    NewFolder,
    NewFile(RemoteEditableFile),
    SelectAll,
    InvertSelection,
    AnalyzeCurrentFolder,
    Refresh,
}

/// The object an action operates on. Keeping this mapping explicit prevents a
/// row menu from silently applying a clicked-row label to the whole selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteContextActionTarget {
    ClickedRow,
    CurrentSelection,
    CurrentFolder,
    CurrentView,
}

impl RemoteContextAction {
    pub(in crate::app) fn target(self) -> RemoteContextActionTarget {
        match self {
            Self::Open
            | Self::OpenWith
            | Self::DownloadTo
            | Self::ToggleFavorite
            | Self::CopyPath
            | Self::AnalyzeDirectory => RemoteContextActionTarget::ClickedRow,
            Self::CopyToClipboard | Self::ProvideForPrograms | Self::Rename | Self::Delete => {
                RemoteContextActionTarget::CurrentSelection
            }
            Self::Paste
            | Self::NewFolder
            | Self::NewFile(_)
            | Self::AnalyzeCurrentFolder
            | Self::Refresh => RemoteContextActionTarget::CurrentFolder,
            Self::SelectAll | Self::InvertSelection => RemoteContextActionTarget::CurrentView,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum RemoteEditableFile {
    Text,
    Markdown,
    Csv,
    Json,
    Html,
    Rust,
}

impl RemoteEditableFile {
    pub(in crate::app) fn details(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Text => ("📄 Textdatei (.txt)", "Neue Textdatei", "txt"),
            Self::Markdown => ("📝 Markdown (.md)", "Neue Notiz", "md"),
            Self::Csv => ("📊 CSV (.csv)", "Neue Tabelle", "csv"),
            Self::Json => ("🔧 JSON (.json)", "Neue Datei", "json"),
            Self::Html => ("🌐 HTML (.html)", "Neue Seite", "html"),
            Self::Rust => ("</> Code (.rs)", "Neue Datei", "rs"),
        }
    }
}

const EDITABLE_FILES: [RemoteEditableFile; 6] = [
    RemoteEditableFile::Text,
    RemoteEditableFile::Markdown,
    RemoteEditableFile::Csv,
    RemoteEditableFile::Json,
    RemoteEditableFile::Html,
    RemoteEditableFile::Rust,
];

pub(in crate::app) fn plan_remote_context_menu(
    subject: RemoteContextSubject,
    capabilities: RemoteContextCapabilities,
) -> Vec<RemoteContextAction> {
    match subject {
        RemoteContextSubject::Row {
            entry_kind,
            selection,
        } => {
            let row_is_dir = entry_kind == RemoteContextEntryKind::Directory;
            let mut actions = vec![RemoteContextAction::Open];
            if !row_is_dir && capabilities.open_with_chooser {
                actions.push(RemoteContextAction::OpenWith);
            }
            actions.push(RemoteContextAction::DownloadTo);
            if selection.includes_clicked() {
                actions.push(RemoteContextAction::CopyToClipboard);
                if capabilities.file_clipboard {
                    actions.push(RemoteContextAction::ProvideForPrograms);
                }
            }
            if selection.is_single() {
                actions.push(RemoteContextAction::Rename);
            }
            if selection.includes_clicked() {
                actions.push(RemoteContextAction::Delete);
            }
            if selection.is_single() && row_is_dir {
                actions.push(RemoteContextAction::ToggleFavorite);
            }
            actions.push(RemoteContextAction::CopyPath);
            actions.push(if row_is_dir {
                RemoteContextAction::AnalyzeDirectory
            } else {
                RemoteContextAction::AnalyzeCurrentFolder
            });
            actions.push(RemoteContextAction::Refresh);
            actions
        }
        RemoteContextSubject::Background => {
            let mut actions = vec![RemoteContextAction::Paste, RemoteContextAction::NewFolder];
            actions.extend(EDITABLE_FILES.map(RemoteContextAction::NewFile));
            actions.extend([
                RemoteContextAction::SelectAll,
                RemoteContextAction::InvertSelection,
                RemoteContextAction::AnalyzeCurrentFolder,
                RemoteContextAction::Refresh,
            ]);
            actions
        }
    }
}
