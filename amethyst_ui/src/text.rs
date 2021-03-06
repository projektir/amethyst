use amethyst_assets::{AssetStorage, Loader};
use amethyst_renderer::{Texture, TextureData, TextureHandle, TextureMetadata};
use gfx::format::{ChannelType, SurfaceType};
use rusttype::{Point, Scale};
use specs::{Component, DenseVecStorage, Fetch, Join, ReadStorage, System, WriteStorage};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::*;

/// A component used to display text in this entities UiTransform
pub struct UiText {
    /// The texture that text is rendered onto.  None if text isn't rendered yet.
    pub(crate) texture: Option<TextureHandle>,
    /// The font used to display the text.
    font: FontHandle, //TODO: Mark this component dirty when the font is reloaded.
    /// The text being displayed
    text: String,
    /// The normalized RGBA color of the text being displayed
    color: [f32; 4],
    /// The font size of the text being displayed
    font_size: f32,
    /// This is true if the texture needs to be re-rendered
    dirty: bool,
}

impl UiText {
    /// Initializes a new UiText
    ///
    /// # Parameters
    ///
    /// * `font`: A handle to a `Font` asset
    /// * `text`: the glyphs to render
    /// * `color`: RGBA color with a maximum of 1.0 and a minimum of 0.0 for each channel
    /// * `font_size`: a uniform scale applied to the glyphs
    pub fn new(font: FontHandle, text: String, color: [f32; 4], font_size: f32) -> UiText {
        UiText {
            texture: None,
            font,
            text,
            color,
            font_size,
            dirty: true,
        }
    }

    /// Retrieves the handle to the font this is using.
    pub fn font(&self) -> &FontHandle {
        &self.font
    }

    /// Sets the font for this to use in rendering.
    ///
    /// Avoid calling this if you don't have to, as calls to this invalidate the text render cache.
    pub fn set_font(&mut self, font: FontHandle) {
        self.font = font;
        self.dirty = true;
    }

    /// Returns the string slice used by this.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Gets a mutable reference to the string to render.
    ///
    /// Avoid calling this if you don't have to, as calls to this invalidate the text render cache.
    pub fn text_mut(&mut self) -> &mut String {
        self.dirty = true;
        &mut self.text
    }

    /// The RGBA color with a maximum of 1.0 and a minimum of 0.0 for each channel of the text.
    pub fn color(&self) -> [f32; 4] {
        self.color
    }

    /// Set the RGBA color with a maximum of 1.0 and a minimum of 0.0 for each channel.
    pub fn set_color(&mut self, color: [f32; 4]) {
        self.color = color;
        self.dirty = true;
    }

    /// The maximum height of the font in pixels.
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Set the maximum height of the font in pixels.
    ///
    /// Avoid calling this if you don't have to, as calls to this invalidate the text render cache.
    pub fn set_font_size(&mut self, size: f32) {
        self.dirty = true;
        self.font_size = size;
    }
}

impl Component for UiText {
    type Storage = DenseVecStorage<Self>;
}

/// This system renders `UiText`.
/// Make sure it's called after changes are made to `UiText` but before
/// the `RenderSystem` from `amethyst_renderer` gets invoked.
pub struct UiTextRenderer;

impl<'a> System<'a> for UiTextRenderer {
    type SystemData = (
        ReadStorage<'a, UiTransform>,
        WriteStorage<'a, UiText>,
        Fetch<'a, Loader>,
        Fetch<'a, AssetStorage<Texture>>,
        Fetch<'a, AssetStorage<FontAsset>>,
    );

    fn run(&mut self, (transform, mut text, loader, tex_storage, font_storage): Self::SystemData) {
        for (transform, text) in (&transform, &mut text)
            .join()
            .filter(|&(_transform, ref text)| text.dirty)
        {
            // TODO: use `TrackedStorage`
            if let Some(font) = font_storage.get(&text.font) {
                text.dirty = false;
                if (*text.text).chars().any(|c| is_combining_mark(c)) {
                    let normalized = text.text.nfd().collect::<String>();
                    text.text = normalized;
                }
                let num_floats = (transform.width * transform.height) as usize * 4;
                let mut render_buffer = vec![0.0; num_floats];
                let height = transform.height as u32;
                let width = transform.width as u32;
                if text.color[3] > 0.01 {
                    for glyph in font.0.layout(
                        &text.text,
                        Scale::uniform(text.font_size),
                        Point::<f32> { x: 0., y: 0. },
                    ) {
                        let position = glyph.position();
                        let pos_x = position.x as u32;
                        glyph.draw(|x, y, v| {
                            if v > 0.01 {
                                let x = x + pos_x;
                                if x < width && y < height {
                                    let start = ((x + y * width) * 4) as usize;
                                    render_buffer[start] = text.color[0];
                                    render_buffer[start + 1] = text.color[1];
                                    render_buffer[start + 2] = text.color[2];
                                    render_buffer[start + 3] = text.color[3] * v;
                                }
                            }
                        });
                    }
                }
                let meta = TextureMetadata {
                    sampler: None,
                    mip_levels: Some(1),
                    size: Some((transform.width as u16, transform.height as u16)),
                    dynamic: false,
                    format: Some(SurfaceType::R32_G32_B32_A32),
                    channel: Some(ChannelType::Float),
                };
                let data = TextureData::F32(render_buffer, meta);
                text.texture = Some(loader.load_from_data(data, (), &tex_storage));
            }
        }
    }
}
