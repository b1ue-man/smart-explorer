use super::super::open_target;
use super::*;
use crate::app::start_request;
use std::cell::RefCell;

#[test]
fn batches_task_folder_remote_locator_survives_launch_and_parse() {
    for locator in [
        "sftp://user@host:22/Ordner mit Leerzeichen/%23#?.txt",
        "ftp://user@host/Ordner/%25name",
        "webdav://host/Ordner/ä #.txt",
    ] {
        let request = folder_location(locator, OpenFoldersWith::App).unwrap();
        let args = RefCell::new(Vec::new());
        open_target::launch(
            &request,
            Path::new("smart_explorer"),
            &|_, values| {
                *args.borrow_mut() = values.to_vec();
                Ok(())
            },
            &|_, _| panic!("remote location reached native file manager"),
        )
        .unwrap();
        let args: Vec<std::ffi::OsString> = args.into_inner().into_iter().map(Into::into).collect();
        assert_eq!(
            start_request::parse(&args).unwrap(),
            StartRequest::OpenLocation {
                locator: locator.into()
            }
        );
        assert!(folder_location(locator, OpenFoldersWith::FileManager).is_err());
    }
}

#[test]
fn batches_task_folder_local_literal_and_file_choice_are_preserved() {
    let folder = Path::new("desktop");
    let name = "Bericht ä # %25";
    let path = folder.join(name);
    assert_eq!(
        desktop_entry(folder, name, true, false, OpenFoldersWith::App).unwrap(),
        OpenRequest::Window(StartRequest::OpenLocation {
            locator: path.to_str().unwrap().into(),
        })
    );
    assert_eq!(
        desktop_entry(folder, name, true, false, OpenFoldersWith::FileManager).unwrap(),
        OpenRequest::Folder(path.clone())
    );
    for with in [OpenFoldersWith::App, OpenFoldersWith::FileManager] {
        for choose in [false, true] {
            assert_eq!(
                desktop_entry(folder, name, false, choose, with).unwrap(),
                OpenRequest::File {
                    path: path.clone(),
                    choose
                }
            );
        }
        assert_eq!(
            desktop_entry(folder, name, true, true, with).unwrap(),
            OpenRequest::File {
                path: path.clone(),
                choose: true
            }
        );
    }
}
