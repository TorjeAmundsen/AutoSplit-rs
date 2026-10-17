//! The Output Preview: the selected split's split image, or its mask on the live capture. While
//! editing, its Mask and Live toggles pick what it shows instead.

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

    /// Whether split `index` has a mask picked, which may still fail to read.
    fn has_mask(&self, index: usize) -> bool {
        let maker = self.maker.borrow();
        maker.shown().and_then(|preset| preset.splits.get(index)).is_some_and(|split| !split.mask.is_empty())
    }

    /// What the Output Preview shows, from its toggles while editing: whether the mask is applied,
    /// and whether it's the live capture rather than the split's grab or screenshot. Otherwise
    /// it's the split image as it would be saved, from the capture until it has a grab or
    /// screenshot.
    fn preview_toggles(&self) -> (bool, bool) {
        let maker = self.maker.borrow();
        if maker.edit.is_some() {
            return (self.windows.maker.get_preview_mask(), self.windows.maker.get_preview_live());
        }
        (true, maker.splits.get(maker.selected).is_some_and(|split| split.source.is_none()))
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
        let (masked, live) = self.preview_toggles();
        // While editing, a split without a mask yet shows the picture as it is
        let mask = || match self.maker_mask(selected) {
            Ok(mask) => Ok(Some(mask)),
            Err(_) if editing && !self.has_mask(selected) => Ok(None),
            Err(error) => Err(error),
        };
        if live {
            // The capture's preview fills it in as frames come, masked when there's a mask
            let message = match if masked { mask().map(drop) } else { Ok(()) } {
                Ok(()) => window.get_live_message(),
                Err(error) => error.into(),
            };
            window.set_preview_message(message);
            return;
        }
        let Some(source) = source else {
            // While editing, the mask itself, when there's nothing to apply it to
            let message = match (masked, self.has_mask(selected)) {
                (true, true) => match self.maker_mask_picture(selected) {
                    Ok(picture) => {
                        window.set_preview_image(picture);
                        return window.set_preview_message(SharedString::new());
                    }
                    Err(error) => error,
                },
                (true, false) => "Choose a mask for this split to see what it keeps.".into(),
                (false, _) => "The split has no grab or screenshot. Turn on Live or Mask to see something.".into(),
            };
            return window.set_preview_message(message.into());
        };
        let shown = (if masked { mask() } else { Ok(None) }).and_then(|mask| {
            let shot = source.image()?;
            Ok(match mask {
                Some(mask) => apply_mask(shot.view(), &mask, &mut ResizeBuffers::default()),
                None => (*shot).clone(),
            })
        });
        match shown {
            Ok(image) => {
                window.set_preview_image(slint::Image::from_rgba8(rgba(image.view())));
                window.set_preview_message(SharedString::new());
            }
            Err(error) => window.set_preview_message(error.into()),
        }
    }

    /// The mask's PNG of split `index` as it is, for the Output Preview while editing.
    fn maker_mask_picture(&self, index: usize) -> Result<slint::Image, String> {
        let maker = self.maker.borrow();
        let Some(mask) = maker.shown().and_then(|preset| Some(preset.splits.get(index)?.mask.clone())) else {
            return Err("There's no preset.".into());
        };
        let path = maker.file(&mask);
        let image = BgraImage::open_png(&path).map_err(|error| format!("Couldn't read {}: {error}", path.display()))?;
        Ok(slint::Image::from_rgba8(rgba(image.view())))
    }

    /// Whether the Output Preview shows the capture: until something's set to make the selected
    /// split's split image from, or while editing with Live on.
    pub(super) fn previews_capture(&self) -> bool {
        self.preview_toggles().1
    }

    /// A frame of the capture's preview: until something's set to make the selected split's
    /// split image from, it's previewed from the capture, masked like a grab would be.
    pub(in crate::gui) fn maker_live_frame(&self, pixels: &Pixels) {
        self.maker.borrow_mut().last_live = Some(pixels.clone());
        if !self.previews_capture() {
            return;
        }
        let window = &self.windows.maker;
        let selected = self.maker.borrow().selected;
        let editing = self.maker.borrow().edit.is_some();
        // While editing, the capture as it is with Mask off, or when the split has no mask yet
        if editing && (!self.preview_toggles().0 || !self.has_mask(selected)) {
            window.set_preview_image(slint::Image::from_rgba8(pixels.clone()));
            return window.set_preview_message(SharedString::new());
        }
        let size = (pixels.width() as usize, pixels.height() as usize);
        if self.maker.borrow().live_mask.as_ref().is_none_or(|(at, _)| *at != size) {
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
