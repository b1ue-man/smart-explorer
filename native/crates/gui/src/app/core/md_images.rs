//! Images of a rendered Markdown file: relative and local paths are read
//! through the place of the document (local file system or its backend),
//! decoded on a background thread and kept as textures; web images and
//! images that cannot be read show their alternative text.
use std::collections::HashMap;
use std::io;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use super::text_tools::TextPlace;

/// State of one image.
pub(in crate::app) enum ImageState {
    Loading,
    Ready(egui::TextureHandle),
    Failed(String),
}

type Loaded = (String, Result<egui::ColorImage, String>);

/// The images of one Markdown view, keyed by resolved path.
pub(in crate::app) struct MdImages {
    images: HashMap<String, ImageState>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl Default for MdImages {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            images: HashMap::new(),
            tx,
            rx,
        }
    }
}

fn read_all(place: &TextPlace, path: &str) -> io::Result<Vec<u8>> {
    let Some(backend) = &place.backend else {
        return std::fs::read(path);
    };
    use crate::textview::TextSource;
    let source = crate::textview::VfsSource::open(backend.clone(), path)?;
    let len = usize::try_from(source.len()).map_err(|_| io::Error::other("Bild zu groß"))?;
    let mut bytes = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = source.read_at(done as u64, &mut bytes[done..])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    bytes.truncate(done);
    Ok(bytes)
}

fn decode(bytes: &[u8], max_side: u32) -> Result<egui::ColorImage, String> {
    let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let image = if image.width() > max_side || image.height() > max_side {
        image.thumbnail(max_side, max_side)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

impl MdImages {
    /// The image at `path`; starts loading it the first time.
    pub(in crate::app) fn get(
        &mut self,
        ctx: &egui::Context,
        place: &TextPlace,
        path: &str,
    ) -> &ImageState {
        if !self.images.contains_key(path) {
            self.images.insert(path.to_string(), ImageState::Loading);
            let tx = self.tx.clone();
            let place = place.clone();
            let key = path.to_string();
            let max_side = ctx.input(|input| input.max_texture_side).min(8192) as u32;
            let repaint = ctx.clone();
            let started = std::thread::Builder::new()
                .name("md-image".into())
                .spawn(move || {
                    let result = read_all(&place, &key)
                        .map_err(|error| error.to_string())
                        .and_then(|bytes| decode(&bytes, max_side.max(1)));
                    let _ = tx.send((key, result));
                    repaint.request_repaint();
                });
            if let Err(error) = started {
                self.images
                    .insert(path.to_string(), ImageState::Failed(error.to_string()));
            }
        }
        &*self
            .images
            .entry(path.to_string())
            .or_insert(ImageState::Loading)
    }

    /// Takes decoded images and turns them into textures.
    pub(in crate::app) fn poll(&mut self, ctx: &egui::Context) {
        for (path, result) in self.rx.try_iter() {
            let state = match result {
                Ok(image) => ImageState::Ready(ctx.load_texture(
                    format!("md-image:{path}"),
                    image,
                    egui::TextureOptions::LINEAR,
                )),
                Err(error) => ImageState::Failed(error),
            };
            self.images.insert(path, state);
        }
    }
}
