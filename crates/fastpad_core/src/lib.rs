use anyhow::{bail, Context, Result};
use fastpad_edit::EditBuffer;
use fastpad_file::{atomic_write, FileHandle, FileMetadata, FileOpenOptions};
use fastpad_search::{SearchEngine, SearchQuery, SearchSummary};
use fastpad_tasks::CancellationToken;
use fastpad_viewport::{ViewAnchor, Viewport, ViewportEngine, ViewportRequest};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DEFAULT_ANALYSIS_THRESHOLD_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_HUGE_EDIT_WARNING_BYTES: u64 = 100 * 1024 * 1024;
pub const DEFAULT_MAX_EDIT_LOAD_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorMode {
    ViewAnalysis,
    Edit,
}

impl EditorMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ViewAnalysis => "View/Analysis Mode",
            Self::Edit => "Edit Mode",
        }
    }

    pub fn is_editable(self) -> bool {
        matches!(self, Self::Edit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub analysis_threshold_bytes: u64,
    pub huge_edit_warning_bytes: u64,
    pub max_edit_load_bytes: u64,
    pub initial_viewport_lines: usize,
    pub initial_viewport_bytes: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            analysis_threshold_bytes: DEFAULT_ANALYSIS_THRESHOLD_BYTES,
            huge_edit_warning_bytes: DEFAULT_HUGE_EDIT_WARNING_BYTES,
            max_edit_load_bytes: DEFAULT_MAX_EDIT_LOAD_BYTES,
            initial_viewport_lines: 120,
            initial_viewport_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct OpenIntent {
    pub force_analysis: bool,
    pub force_edit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeDecision {
    pub mode: EditorMode,
    pub reason: String,
    pub requires_huge_edit_warning: bool,
}

pub struct ModeManager {
    settings: AppSettings,
}

impl ModeManager {
    pub fn new(settings: AppSettings) -> Self {
        Self { settings }
    }

    pub fn choose_for_open(&self, metadata: &FileMetadata, intent: OpenIntent) -> ModeDecision {
        if intent.force_analysis {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                reason: "forced read-only analysis".into(),
                requires_huge_edit_warning: false,
            };
        }

        if intent.force_edit {
            return ModeDecision {
                mode: EditorMode::Edit,
                reason: "forced edit".into(),
                requires_huge_edit_warning: metadata.len >= self.settings.huge_edit_warning_bytes,
            };
        }

        if metadata.len >= self.settings.analysis_threshold_bytes
            || metadata.intelligence.very_long_line_warning
            || !metadata.intelligence.likely_text
        {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                reason: "large or risky file opened using bounded read-only path".into(),
                requires_huge_edit_warning: false,
            };
        }

        ModeDecision {
            mode: EditorMode::Edit,
            reason: "file is within edit threshold".into(),
            requires_huge_edit_warning: false,
        }
    }
}

pub enum DocumentBacking {
    View {
        file: FileHandle,
        viewport: ViewportEngine,
    },
    Edit {
        buffer: EditBuffer,
        path: Option<PathBuf>,
        metadata: Option<FileMetadata>,
    },
}

pub struct Document {
    id: DocumentId,
    title: String,
    mode: EditorMode,
    mode_reason: String,
    backing: DocumentBacking,
}

impl Document {
    pub fn untitled(id: DocumentId) -> Self {
        Self {
            id,
            title: "Untitled".into(),
            mode: EditorMode::Edit,
            mode_reason: "new document".into(),
            backing: DocumentBacking::Edit {
                buffer: EditBuffer::new(),
                path: None,
                metadata: None,
            },
        }
    }

    pub fn open(
        id: DocumentId,
        path: impl AsRef<Path>,
        settings: &AppSettings,
        intent: OpenIntent,
    ) -> Result<Self> {
        let file = FileHandle::open(path.as_ref(), FileOpenOptions::default())?;
        let decision = ModeManager::new(settings.clone()).choose_for_open(file.metadata(), intent);
        let title = file
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_string();

        match decision.mode {
            EditorMode::ViewAnalysis => Ok(Self {
                id,
                title,
                mode: EditorMode::ViewAnalysis,
                mode_reason: decision.reason,
                backing: DocumentBacking::View {
                    file,
                    viewport: ViewportEngine::default(),
                },
            }),
            EditorMode::Edit => {
                let bytes = file
                    .read_entire_if_under(settings.max_edit_load_bytes)
                    .with_context(|| format!("load {}", file.path().display()))?
                    .with_context(|| {
                        format!("{} exceeds max edit load threshold", file.path().display())
                    })?;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let metadata = file.metadata().clone();
                Ok(Self {
                    id,
                    title,
                    mode: EditorMode::Edit,
                    mode_reason: decision.reason,
                    backing: DocumentBacking::Edit {
                        buffer: EditBuffer::from_text(&text),
                        path: Some(file.path().to_path_buf()),
                        metadata: Some(metadata),
                    },
                })
            }
        }
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn mode(&self) -> EditorMode {
        self.mode
    }

    pub fn mode_reason(&self) -> &str {
        &self.mode_reason
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.backing {
            DocumentBacking::View { file, .. } => Some(file.path()),
            DocumentBacking::Edit { path, .. } => path.as_deref(),
        }
    }

    pub fn metadata(&self) -> Option<&FileMetadata> {
        match &self.backing {
            DocumentBacking::View { file, .. } => Some(file.metadata()),
            DocumentBacking::Edit { metadata, .. } => metadata.as_ref(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backing {
            DocumentBacking::View { .. } => false,
            DocumentBacking::Edit { buffer, .. } => buffer.is_dirty(),
        }
    }

    pub fn initial_viewport(&mut self, settings: &AppSettings) -> Result<Viewport> {
        self.viewport(ViewportRequest {
            anchor: ViewAnchor::Start,
            max_lines: settings.initial_viewport_lines,
            max_bytes: settings.initial_viewport_bytes,
        })
    }

    pub fn viewport(&mut self, request: ViewportRequest) -> Result<Viewport> {
        match &mut self.backing {
            DocumentBacking::View { file, viewport } => viewport.render(file, request),
            DocumentBacking::Edit { buffer, .. } => {
                let text = buffer.text();
                let lines = text
                    .lines()
                    .take(request.max_lines)
                    .enumerate()
                    .map(|(idx, line)| fastpad_line_index::LineSlice {
                        line_number: Some(idx as u64),
                        start: fastpad_file::ByteOffset(0),
                        end: fastpad_file::ByteOffset(0),
                        text: line.to_string(),
                        truncated: false,
                    })
                    .collect();
                Ok(Viewport {
                    anchor: request.anchor,
                    start: fastpad_file::ByteOffset(0),
                    end: fastpad_file::ByteOffset(text.len() as u64),
                    file_len: text.len() as u64,
                    lines,
                })
            }
        }
    }

    pub fn edit_buffer_mut(&mut self) -> Result<&mut EditBuffer> {
        match &mut self.backing {
            DocumentBacking::Edit { buffer, .. } => Ok(buffer),
            DocumentBacking::View { .. } => bail!("document is in View/Analysis Mode"),
        }
    }

    pub fn full_text_for_editing(&self) -> Result<String> {
        match &self.backing {
            DocumentBacking::Edit { buffer, .. } => Ok(buffer.text()),
            DocumentBacking::View { .. } => bail!("document is in View/Analysis Mode"),
        }
    }

    pub fn search(&self, query: &SearchQuery, cancel: &CancellationToken) -> Result<SearchSummary> {
        match &self.backing {
            DocumentBacking::View { file, .. } => SearchEngine::search(file, query, cancel),
            DocumentBacking::Edit { buffer, .. } => {
                let text = buffer.text();
                SearchEngine::search_bytes(text.as_bytes(), query, cancel)
            }
        }
    }

    pub fn save(&mut self) -> Result<()> {
        match &mut self.backing {
            DocumentBacking::View { .. } => bail!("cannot save a read-only analysis document"),
            DocumentBacking::Edit { buffer, path, .. } => {
                let Some(path) = path else {
                    bail!("save-as path required for untitled document");
                };
                atomic_write(path, buffer.text().as_bytes())?;
                buffer.mark_clean();
                Ok(())
            }
        }
    }

    pub fn status_line(&self) -> String {
        let dirty = if self.is_dirty() { " modified" } else { "" };
        let size = self
            .metadata()
            .map(|metadata| format!(" {} bytes", metadata.len))
            .unwrap_or_default();
        format!("{}{} - {}{}", self.title, dirty, self.mode.label(), size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub u64);

#[derive(Default)]
pub struct DocumentManager {
    settings: AppSettings,
    next_id: u64,
    documents: BTreeMap<DocumentId, Arc<RwLock<Document>>>,
    active: Option<DocumentId>,
}

impl DocumentManager {
    pub fn new(settings: AppSettings) -> Self {
        Self {
            settings,
            next_id: 1,
            documents: BTreeMap::new(),
            active: None,
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn open(&mut self, path: impl AsRef<Path>, intent: OpenIntent) -> Result<DocumentId> {
        let id = self.allocate_id();
        let doc = Document::open(id, path, &self.settings, intent)?;
        self.documents.insert(id, Arc::new(RwLock::new(doc)));
        self.active = Some(id);
        Ok(id)
    }

    pub fn new_untitled(&mut self) -> DocumentId {
        let id = self.allocate_id();
        let doc = Document::untitled(id);
        self.documents.insert(id, Arc::new(RwLock::new(doc)));
        self.active = Some(id);
        id
    }

    pub fn get(&self, id: DocumentId) -> Option<Arc<RwLock<Document>>> {
        self.documents.get(&id).cloned()
    }

    pub fn active(&self) -> Option<Arc<RwLock<Document>>> {
        self.active.and_then(|id| self.get(id))
    }

    fn allocate_id(&mut self) -> DocumentId {
        let id = DocumentId(self.next_id);
        self.next_id += 1;
        id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CommandId {
    Open,
    Save,
    Search,
    Copy,
    InsertText,
    DeleteSelection,
    Replace,
    ConvertToEditMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCapability {
    Always,
    RequiresEdit,
    RequiresViewOrEdit,
}

#[derive(Debug, Clone)]
pub struct CommandRegistry {
    capabilities: BTreeMap<CommandId, CommandCapability>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        let mut capabilities = BTreeMap::new();
        capabilities.insert(CommandId::Open, CommandCapability::Always);
        capabilities.insert(CommandId::Save, CommandCapability::RequiresEdit);
        capabilities.insert(CommandId::Search, CommandCapability::RequiresViewOrEdit);
        capabilities.insert(CommandId::Copy, CommandCapability::RequiresViewOrEdit);
        capabilities.insert(CommandId::InsertText, CommandCapability::RequiresEdit);
        capabilities.insert(CommandId::DeleteSelection, CommandCapability::RequiresEdit);
        capabilities.insert(CommandId::Replace, CommandCapability::RequiresEdit);
        capabilities.insert(
            CommandId::ConvertToEditMode,
            CommandCapability::RequiresViewOrEdit,
        );
        Self { capabilities }
    }
}

impl CommandRegistry {
    pub fn is_enabled(&self, command: CommandId, mode: EditorMode) -> bool {
        match self.capabilities.get(&command).copied() {
            Some(CommandCapability::Always | CommandCapability::RequiresViewOrEdit) => true,
            Some(CommandCapability::RequiresEdit) => mode.is_editable(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn chooses_analysis_for_large_files() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "small").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let settings = AppSettings {
            analysis_threshold_bytes: 1,
            ..Default::default()
        };
        let decision =
            ModeManager::new(settings).choose_for_open(file.metadata(), OpenIntent::default());

        assert_eq!(decision.mode, EditorMode::ViewAnalysis);
    }

    #[test]
    fn disables_edit_commands_in_analysis_mode() {
        let registry = CommandRegistry::default();
        assert!(!registry.is_enabled(CommandId::InsertText, EditorMode::ViewAnalysis));
        assert!(registry.is_enabled(CommandId::Search, EditorMode::ViewAnalysis));
    }

    #[test]
    fn edit_documents_expose_full_text_for_native_text_view() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for idx in 0..150 {
            writeln!(tmp, "line {idx}").unwrap();
        }
        let settings = AppSettings::default();
        let doc =
            Document::open(DocumentId(1), tmp.path(), &settings, OpenIntent::default()).unwrap();

        assert_eq!(doc.mode(), EditorMode::Edit);
        assert_eq!(doc.full_text_for_editing().unwrap().lines().count(), 150);
    }
}
