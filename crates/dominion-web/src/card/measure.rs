//! Text widths for fitting: measured with a canvas in the browser, in the same fonts the
//! card is drawn with (whichever of them have loaded); estimated natively.

use super::fit::{Approx, Font, Measure};

/// The font stacks of cards.css, at 100px.
#[cfg_attr(not(all(feature = "web", target_arch = "wasm32")), allow(dead_code))]
pub fn canvas_font(font: Font) -> &'static str {
    match font {
        Font::Body => "100px DomBody, 'Times New Roman', 'Liberation Serif', serif",
        Font::BodyBold => "bold 100px DomBody, 'Times New Roman', 'Liberation Serif', serif",
        Font::BodyItalic => "italic 100px DomBody, 'Times New Roman', 'Liberation Serif', serif",
        Font::Number => "bold 100px DomNum, 'Times New Roman', 'Liberation Serif', serif",
        Font::Title => "100px DomTitle, 'Times New Roman', 'Liberation Serif', serif",
    }
}

/// The measurer for the current fonts; `generation` changes when fonts load.
pub fn measurer(generation: u32) -> Box<dyn Measure> {
    imp::measurer(generation)
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod imp {
    use super::{Approx, Font, Measure, canvas_font};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use wasm_bindgen::JsCast;
    use web_sys::CanvasRenderingContext2d;

    /// Widths by font and text, for one font generation.
    type Widths = (u32, HashMap<(Font, String), f32>);

    thread_local! {
        static CTX: RefCell<Option<CanvasRenderingContext2d>> = const { RefCell::new(None) };
        static CACHE: RefCell<Widths> = RefCell::new((0, HashMap::new()));
    }

    fn context() -> Option<CanvasRenderingContext2d> {
        CTX.with(|c| {
            if c.borrow().is_none() {
                let ctx = web_sys::window()?
                    .document()?
                    .create_element("canvas")
                    .ok()?
                    .dyn_into::<web_sys::HtmlCanvasElement>()
                    .ok()?
                    .get_context("2d")
                    .ok()??
                    .dyn_into::<CanvasRenderingContext2d>()
                    .ok()?;
                *c.borrow_mut() = Some(ctx);
            }
            c.borrow().clone()
        })
    }

    struct Canvas {
        ctx: CanvasRenderingContext2d,
        generation: u32,
    }

    impl Measure for Canvas {
        fn width(&self, text: &str, font: Font) -> f32 {
            CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.0 != self.generation {
                    *cache = (self.generation, HashMap::new());
                }
                if let Some(w) = cache.1.get(&(font, text.to_string())) {
                    return *w;
                }
                self.ctx.set_font(canvas_font(font));
                let w = self
                    .ctx
                    .measure_text(text)
                    .map(|m| m.width() as f32 / 100.0)
                    .unwrap_or_else(|_| Approx.width(text, font));
                cache.1.insert((font, text.to_string()), w);
                w
            })
        }
    }

    pub fn measurer(generation: u32) -> Box<dyn Measure> {
        match context() {
            Some(ctx) => Box::new(Canvas { ctx, generation }),
            None => Box::new(Approx),
        }
    }
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
mod imp {
    use super::{Approx, Measure};

    pub fn measurer(_generation: u32) -> Box<dyn Measure> {
        Box::new(Approx)
    }
}
