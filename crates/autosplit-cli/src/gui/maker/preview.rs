//! The Output Preview: the selected split's split image, or its mask on the live capture.

use super::*;

impl Gui {
    /// The mask of split `index`, read once.
    pub(super) fn maker_mask(&self, index: usize) -> Result<Arc<Mask>, String> {
        let mut maker = self.maker.borrow_mut();
        let Some(mask) = maker.shown().and_then(|preset| Some(preset.splits.get(index)?.mask.clone())) else {
            return Err("There's no preset.".into());
        };
        if mask.is_empty() {
            return Err("Choose a mask for this split to see what it keeps.".into());
        }
        let path = maker.file(&mask);
        if let Some(mask) = maker.masks.get(&path) {
            return Ok(mask.clone());
        }
        let mask = Arc::new(Mask::open(&path).map_err(|error| format!("Couldn't read {}: {error}", path.display()))?);
        maker.masks.insert(path, mask.clone());
        Ok(mask)
    }

    /// The selected split's split image, or its mask when nothing's set to make it from.
    pub(super) fn show_maker_preview(&self) {
        let maker = self.maker.borrow();
        let selected = maker.selected;
        let source = maker.source(selected).cloned();
        let has_split = maker.shown().is_some_and(|preset| selected < preset.splits.len());
        let (editing, no_presets) = (maker.edit.is_some(), maker.presets.is_empty());
        drop(maker);
        let window = &self.windows.maker;
        if !has_split {
            let message = match (editing, no_presets) {
                (true, _) => "Add a split.",
                (false, true) => "There are no presets.",
                (false, false) => "The preset has no splits.",
            };
            return window.set_preview_message(message.into());
        }
        let Some(source) = source else {
            // The capture's preview fills it in, masked, as frames come, once there's a mask
            let message = match self.maker_mask(selected) {
                Ok(_) => window.get_live_message(),
                Err(error) => error.into(),
            };
            window.set_preview_message(message);
            return;
        };
        let shown = self.maker_mask(selected).and_then(|mask| {
            let shot = source.image()?;
            Ok(apply_mask(shot.view(), &mask, &mut ResizeBuffers::default()))
        });
        match shown {
            Ok(image) => {
                window.set_preview_image(slint::Image::from_rgba8(rgba(image.view())));
                window.set_preview_message(SharedString::new());
            }
            Err(error) => window.set_preview_message(error.into()),
        }
    }

    /// Whether the selected split's split image is previewed from the capture: nothing's set to
    /// make it from yet.
    pub(super) fn previews_capture(&self) -> bool {
        let maker = self.maker.borrow();
        maker.splits.get(maker.selected).is_some_and(|split| split.source.is_none())
    }

    /// A frame of the capture's preview: until something's set to make the selected split's
    /// split image from, it's previewed from the capture, masked like a grab would be.
    pub(in crate::gui) fn maker_live_frame(&self, pixels: &Pixels) {
        if !self.previews_capture() {
            return;
        }
        let size = (pixels.width() as usize, pixels.height() as usize);
        if self.maker.borrow().live_mask.as_ref().is_none_or(|(at, _)| *at != size) {
            let selected = self.maker.borrow().selected;
            let Ok(mask) = self.maker_mask(selected) else { return };
            let alpha = mask.scaled(size.0, size.1, &mut ResizeBuffers::default());
            self.maker.borrow_mut().live_mask = Some((size, alpha));
        }
        let maker = self.maker.borrow();
        let Some((_, alpha)) = &maker.live_mask else { return };
        let mut masked = pixels.clone();
        for (pixel, &alpha) in masked.make_mut_slice().iter_mut().zip(alpha) {
            if alpha != 255 {
                *pixel = Default::default();
            }
        }
        drop(maker);
        let window = &self.windows.maker;
        window.set_preview_image(slint::Image::from_rgba8(masked));
        window.set_preview_message(SharedString::new());
    }

    /// The capture's preview shows a message instead, which the split image's preview shows too
    /// while it's previewed from the capture.
    pub(in crate::gui) fn maker_live_message(&self, message: &str) {
        if self.previews_capture() {
            self.windows.maker.set_preview_message(message.into());
        }
    }
}
