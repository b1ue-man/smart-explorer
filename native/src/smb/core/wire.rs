//! SMB2 operations on one tree through smb2's public message API
//! (`Connection::execute`/`execute_compound`). Stat and delete open the entry
//! itself with FILE_OPEN_REPARSE_POINT, so a link is described and removed
//! as a link and its target is never touched; a reparse point's tag decides
//! whether it is a link or a data file (`listing::is_data_reparse_tag`).
use super::listing::{self, Attributes};
use super::url::host_of;
use crate::vfs::VfsMeta;
use smb2::client::Connection;
use smb2::msg::close::CloseRequest;
use smb2::msg::create::{
    CreateDisposition, CreateRequest, CreateResponse, ImpersonationLevel, ShareAccess,
};
use smb2::msg::query_directory::{
    FileInformationClass, QueryDirectoryFlags, QueryDirectoryRequest, QueryDirectoryResponse,
};
use smb2::msg::query_info::{QueryInfoRequest, QueryInfoResponse};
use smb2::msg::set_info::{InfoType, SetInfoRequest};
use smb2::pack::{ReadCursor, Unpack};
use smb2::types::flags::FileAccessMask;
use smb2::types::status::NtStatus;
use smb2::types::{Command, FileId, OplockLevel};
use smb2::{CompoundOp, Error, Frame, Tree};

/// Create options (MS-SMB2 2.2.13).
pub(super) const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
pub(super) const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
pub(super) const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// FileDispositionInformation (MS-FSCC 2.4.11), `DeletePending = 1`.
const FILE_DISPOSITION_INFORMATION: u8 = 13;
/// FileAttributeTagInformation (MS-FSCC 2.4.6): attributes + reparse tag.
const FILE_ATTRIBUTE_TAG_INFORMATION: u8 = 35;
const ATTRIBUTE_TAG_LEN: u32 = 8;
/// One credit per QUERY_DIRECTORY (MS-SMB2 3.2.4.1.5).
const QUERY_BUFFER_LEN: u32 = 65_536;

pub(super) fn protocol(status: NtStatus, command: Command) -> Error {
    Error::Protocol { status, command }
}

/// The CREATE name of a share-relative path. A DFS-capable share expects
/// `server\share\path` (MS-SMB2 3.2.4.3, like smb2's `Tree::format_path`);
/// referrals themselves are never followed. `tree.server` is `host:port`,
/// as `SmbClient::connect_share` sets it.
pub(super) fn wire_path(tree: &Tree, rel: &str) -> String {
    let encoded = smb2::encode_path(rel);
    if !tree.is_dfs {
        return encoded;
    }
    let host = host_of(&tree.server);
    if encoded.is_empty() {
        format!("{host}\\{}", tree.share_name)
    } else {
        format!("{host}\\{}\\{encoded}", tree.share_name)
    }
}

/// An open of an existing entry with every share mode granted.
pub(super) fn open_request(tree: &Tree, rel: &str, access: u32, options: u32) -> CreateRequest {
    CreateRequest {
        requested_oplock_level: OplockLevel::None,
        impersonation_level: ImpersonationLevel::Impersonation,
        desired_access: FileAccessMask::new(access),
        file_attributes: 0,
        share_access: ShareAccess(
            ShareAccess::FILE_SHARE_READ
                | ShareAccess::FILE_SHARE_WRITE
                | ShareAccess::FILE_SHARE_DELETE,
        ),
        create_disposition: CreateDisposition::FileOpen,
        create_options: options,
        name: wire_path(tree, rel),
        create_contexts: Vec::new(),
    }
}

/// A CLOSE for the handle a preceding CREATE in the same compound opened.
pub(super) fn related_close() -> CloseRequest {
    CloseRequest {
        flags: 0,
        file_id: FileId::SENTINEL,
    }
}

/// A SET_INFO(FileInformation) request; `FileId::SENTINEL` inside a compound.
pub(super) fn set_file_info(class: u8, file_id: FileId, buffer: Vec<u8>) -> SetInfoRequest {
    SetInfoRequest {
        info_type: InfoType::File,
        file_info_class: class,
        additional_information: 0,
        file_id,
        buffer,
    }
}

/// One frame per operation, or the first waiter-level error.
pub(super) fn frames(
    results: Vec<smb2::Result<Frame>>,
    expected: usize,
) -> smb2::Result<Vec<Frame>> {
    let frames = results.into_iter().collect::<smb2::Result<Vec<Frame>>>()?;
    if frames.len() != expected {
        return Err(Error::invalid_data(format!(
            "compound response has {} frames, expected {expected}",
            frames.len()
        )));
    }
    Ok(frames)
}

pub(super) fn created(frame: &Frame) -> smb2::Result<CreateResponse> {
    if frame.header.status != NtStatus::SUCCESS {
        return Err(protocol(frame.header.status, Command::Create));
    }
    CreateResponse::unpack(&mut ReadCursor::new(&frame.body))
}

/// Attributes from a CREATE response, which carries no reparse tag.
pub(super) fn attributes_of(created: &CreateResponse) -> Attributes {
    Attributes {
        attributes: created.file_attributes,
        size: created.end_of_file,
        last_write: created.last_write_time.0,
        creation: created.creation_time.0,
        reparse_tag: None,
    }
}

/// QUERY_INFO(FileAttributeTagInformation) for `file_id` (the sentinel
/// inside a compound).
fn attribute_tag_request(file_id: FileId) -> QueryInfoRequest {
    QueryInfoRequest {
        info_type: InfoType::File,
        file_info_class: FILE_ATTRIBUTE_TAG_INFORMATION,
        output_buffer_length: ATTRIBUTE_TAG_LEN,
        additional_information: 0,
        flags: 0,
        file_id,
        input_buffer: Vec::new(),
    }
}

/// The reparse tag of a QUERY_INFO answer; `None` when the server refused
/// or answered something unreadable (the entry then stays link-like).
fn reparse_tag_of(frame: &Frame) -> Option<u32> {
    if frame.header.status != NtStatus::SUCCESS {
        return None;
    }
    let response = QueryInfoResponse::unpack(&mut ReadCursor::new(&frame.body)).ok()?;
    listing::attribute_tag(&response.output_buffer)
}

/// Best effort: a handle whose compound CLOSE failed must not leak.
pub(super) async fn close_quietly(conn: &Connection, tree: &Tree, file_id: FileId) {
    let mut conn = conn.clone();
    let _ = tree.close_handle(&mut conn, file_id).await;
}

/// Metadata of the entry itself (a link is not followed): CREATE +
/// QUERY_INFO(FileAttributeTagInformation) + CLOSE in one round trip. The
/// CREATE response carries attributes, times and size, the query the reparse
/// tag. A failed query only leaves the tag unknown (and the CLOSE behind it
/// cascades, so the handle is then closed by hand).
pub(super) async fn stat(conn: &Connection, tree: &Tree, rel: &str) -> smb2::Result<Attributes> {
    let create = open_request(
        tree,
        rel,
        FileAccessMask::FILE_READ_ATTRIBUTES | FileAccessMask::SYNCHRONIZE,
        FILE_OPEN_REPARSE_POINT,
    );
    let query = attribute_tag_request(FileId::SENTINEL);
    let close = related_close();
    let ops = [
        CompoundOp::new(Command::Create, &create, Some(tree.tree_id)),
        CompoundOp::new(Command::QueryInfo, &query, Some(tree.tree_id)),
        CompoundOp::new(Command::Close, &close, Some(tree.tree_id)),
    ];
    let frames = frames(conn.execute_compound(&ops).await?, ops.len())?;
    let response = created(&frames[0])?;
    if frames[2].header.status != NtStatus::SUCCESS {
        close_quietly(conn, tree, response.file_id).await;
    }
    let mut attributes = attributes_of(&response);
    if attributes.is_reparse_point() {
        attributes.reparse_tag = reparse_tag_of(&frames[1]);
    }
    Ok(attributes)
}

/// Whether a QUERY_DIRECTORY status ends the listing. Servers without `.`
/// and `..` answer the first query on an empty folder with NO_SUCH_FILE
/// (the CREATE already proved the folder exists).
pub(super) fn ends_listing(status: NtStatus, first_query: bool) -> bool {
    status == NtStatus::NO_MORE_FILES || (first_query && status == NtStatus::NO_SUCH_FILE)
}

/// A directory listing: CREATE, QUERY_DIRECTORY until NO_MORE_FILES, CLOSE.
/// Opening follows a link to a directory (browsing into it is allowed);
/// the entries keep their own reparse attribute.
pub(super) async fn list(conn: &Connection, tree: &Tree, rel: &str) -> smb2::Result<Vec<VfsMeta>> {
    let open = open_request(
        tree,
        rel,
        FileAccessMask::FILE_READ_DATA
            | FileAccessMask::FILE_READ_ATTRIBUTES
            | FileAccessMask::SYNCHRONIZE,
        FILE_DIRECTORY_FILE,
    );
    let frame = conn
        .execute(Command::Create, &open, Some(tree.tree_id))
        .await?;
    let file_id = created(&frame)?.file_id;
    let listed = query_all(conn, tree, file_id).await;
    let mut close_conn = conn.clone();
    let closed = tree.close_handle(&mut close_conn, file_id).await;
    let entries = listed?;
    closed?;
    Ok(entries)
}

async fn query_all(conn: &Connection, tree: &Tree, file_id: FileId) -> smb2::Result<Vec<VfsMeta>> {
    let buffer_len = conn
        .params()
        .map(|params| params.max_transact_size.min(QUERY_BUFFER_LEN))
        .unwrap_or(QUERY_BUFFER_LEN);
    let mut entries = Vec::new();
    let mut restart = true;
    loop {
        let request = QueryDirectoryRequest {
            file_information_class: FileInformationClass::FileBothDirectoryInformation,
            flags: QueryDirectoryFlags(if restart {
                QueryDirectoryFlags::RESTART_SCANS
            } else {
                0
            }),
            file_index: 0,
            file_id,
            output_buffer_length: buffer_len,
            file_name: "*".to_string(),
        };
        let frame = conn
            .execute(Command::QueryDirectory, &request, Some(tree.tree_id))
            .await?;
        if ends_listing(frame.header.status, restart) {
            return Ok(entries);
        }
        if frame.header.status != NtStatus::SUCCESS {
            return Err(protocol(frame.header.status, Command::QueryDirectory));
        }
        let response = QueryDirectoryResponse::unpack(&mut ReadCursor::new(&frame.body))?;
        if response.output_buffer.is_empty() {
            return Ok(entries);
        }
        entries.extend(
            listing::parse_directory_info(&response.output_buffer).map_err(Error::invalid_data)?,
        );
        restart = false;
    }
}

/// CREATE + SET_INFO + CLOSE in one round trip. A failure after a successful
/// CREATE closes the handle by hand (a cascaded CLOSE fails with the op
/// before it). `close_must_succeed` = the change only takes effect on CLOSE.
pub(super) async fn create_set_close(
    conn: &Connection,
    tree: &Tree,
    create: &CreateRequest,
    set: &SetInfoRequest,
    close_must_succeed: bool,
) -> smb2::Result<()> {
    let close = related_close();
    let ops = [
        CompoundOp::new(Command::Create, create, Some(tree.tree_id)),
        CompoundOp::new(Command::SetInfo, set, Some(tree.tree_id)),
        CompoundOp::new(Command::Close, &close, Some(tree.tree_id)),
    ];
    let frames = frames(conn.execute_compound(&ops).await?, ops.len())?;
    let response = created(&frames[0])?;
    let set_status = frames[1].header.status;
    if set_status != NtStatus::SUCCESS {
        close_quietly(conn, tree, response.file_id).await;
        return Err(protocol(set_status, Command::SetInfo));
    }
    let close_status = frames[2].header.status;
    if close_status != NtStatus::SUCCESS {
        close_quietly(conn, tree, response.file_id).await;
        if close_must_succeed {
            return Err(protocol(close_status, Command::Close));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeleteKind {
    File,
    Directory,
}

/// Deletes the entry itself through SET_INFO(FileDispositionInformation);
/// smb2 documents why not FILE_DELETE_ON_CLOSE (Samba reports a non-empty
/// folder as deleted). A link to a folder is refused as a "file" by
/// FILE_NON_DIRECTORY_FILE and removed through [`delete_link_dir`].
pub(super) async fn delete(
    conn: &Connection,
    tree: &Tree,
    rel: &str,
    kind: DeleteKind,
) -> smb2::Result<()> {
    let type_option = match kind {
        DeleteKind::File => FILE_NON_DIRECTORY_FILE,
        DeleteKind::Directory => FILE_DIRECTORY_FILE,
    };
    let create = open_request(
        tree,
        rel,
        FileAccessMask::DELETE | FileAccessMask::FILE_READ_ATTRIBUTES,
        FILE_OPEN_REPARSE_POINT | type_option,
    );
    let set = set_file_info(FILE_DISPOSITION_INFORMATION, FileId::SENTINEL, vec![1]);
    match create_set_close(conn, tree, &create, &set, true).await {
        Err(error)
            if kind == DeleteKind::File
                && error.status() == Some(NtStatus::FILE_IS_A_DIRECTORY) =>
        {
            delete_link_dir(conn, tree, rel).await
        }
        result => result,
    }
}

/// A folder-type link (directory symlink, junction, any reparse point that
/// is not a data one) is deleted as the link it is; a real folder, including
/// a data reparse folder such as a cloud placeholder, stays a "file is a
/// directory" refusal. The type check and the delete use the same handle, so
/// nothing can be swapped in between.
async fn delete_link_dir(conn: &Connection, tree: &Tree, rel: &str) -> smb2::Result<()> {
    let open = open_request(
        tree,
        rel,
        FileAccessMask::DELETE | FileAccessMask::FILE_READ_ATTRIBUTES,
        FILE_OPEN_REPARSE_POINT,
    );
    let frame = conn
        .execute(Command::Create, &open, Some(tree.tree_id))
        .await?;
    let response = created(&frame)?;
    let mut attributes = attributes_of(&response);
    if attributes.is_reparse_point() {
        let query = attribute_tag_request(response.file_id);
        let answer = conn
            .execute(Command::QueryInfo, &query, Some(tree.tree_id))
            .await;
        attributes.reparse_tag = answer.ok().as_ref().and_then(reparse_tag_of);
    }
    if !attributes.is_link() {
        close_quietly(conn, tree, response.file_id).await;
        return Err(protocol(NtStatus::FILE_IS_A_DIRECTORY, Command::Create));
    }
    let set = set_file_info(FILE_DISPOSITION_INFORMATION, response.file_id, vec![1]);
    let marked = conn
        .execute(Command::SetInfo, &set, Some(tree.tree_id))
        .await;
    let mut close_conn = conn.clone();
    let closed = tree.close_handle(&mut close_conn, response.file_id).await;
    let marked = marked?;
    if marked.header.status != NtStatus::SUCCESS {
        return Err(protocol(marked.header.status, Command::SetInfo));
    }
    closed
}
