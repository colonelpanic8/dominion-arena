//! The server's asset cache (`/assets/manifest.json`): card art, frame templates,
//! icons and fonts, referenced by URL only. Without a cache (no manifest) cards are
//! drawn in CSS instead.

use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::assets::{CardAssets, Manifest};
use std::collections::BTreeSet;
use std::rc::Rc;

/// The fonts cards and the table use, by the family names the stylesheets give them.
const FONTS: [(&str, &str, &str); 7] = [
    ("DomTitle", "400", "fonts/TrajanPro3SemiBold.ttf"),
    ("DomBody", "400", "fonts/times.ttf"),
    ("DomBody", "700", "fonts/timesbd.ttf"),
    ("DomNum", "700", "fonts/MinionPro-Bold.woff"),
    ("DomCount", "400", "fonts/Montserrat-SemiBold.ttf"),
    ("DomUi", "400", "fonts/Raleway-Regular.ttf"),
    ("DomUi", "700", "fonts/Raleway-Bold.ttf"),
];

/// Icons cards.css draws through `--dc-<name>`, with a CSS stand-in when absent.
const ICONS: [(&str, &str); 6] = [
    ("coin", "images/elements/coin.png"),
    ("debt", "images/elements/debt_130.png"),
    ("potion", "images/elements/potion.png"),
    ("vp", "images/elements/victory_150.png"),
    ("sun", "images/elements/rsun.png"),
    ("back", "images/cards/templates/back.jpg"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Loading,
    Loaded,
    /// The server has no asset cache.
    Absent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssetDb {
    pub status: Status,
    manifest: Option<Rc<Manifest>>,
    files: Rc<BTreeSet<String>>,
    /// Bumped when the card fonts have loaded, so text is measured again.
    pub fonts: u32,
}

/// The URL of a path in the asset cache.
pub fn url(path: &str) -> String {
    format!("/assets/{path}")
}

impl AssetDb {
    pub fn new(manifest: Manifest) -> AssetDb {
        let files = manifest.files.iter().cloned().collect();
        AssetDb {
            status: Status::Loaded,
            manifest: Some(Rc::new(manifest)),
            files: Rc::new(files),
            fonts: 0,
        }
    }

    pub fn absent() -> AssetDb {
        AssetDb {
            status: Status::Absent,
            ..AssetDb::default()
        }
    }

    pub fn manifest(&self) -> Option<&Manifest> {
        self.manifest.as_deref()
    }

    /// A card's files, by engine card name.
    pub fn card(&self, name: &str) -> Option<&CardAssets> {
        self.manifest.as_ref()?.cards.get(name)
    }

    /// Whether the cache has a file other than a card's own.
    pub fn has(&self, path: &str) -> bool {
        self.files.contains(path)
    }

    /// The URL of a cached file, if the cache has it.
    pub fn file_url(&self, path: &str) -> Option<String> {
        self.has(path).then(|| url(path))
    }

    /// Whether names can be set in the title font; without it they are capitals.
    pub fn has_title_font(&self) -> bool {
        self.has(FONTS[0].2)
    }

    /// `@font-face` rules and icon variables for what the cache has.
    pub fn css(&self) -> String {
        let mut css = String::new();
        for (family, weight, path) in FONTS {
            if self.has(path) {
                css.push_str(&format!(
                    "@font-face{{font-family:{family};font-weight:{weight};font-display:block;src:url('{}');}}\n",
                    url(path)
                ));
            }
        }
        css.push_str(":root{");
        for (name, path) in ICONS {
            if self.has(path) {
                css.push_str(&format!("--dc-{name}:url('{}');", url(path)));
            }
        }
        css.push_str("}\n");
        for (name, path) in [
            ("debt", "images/elements/debt_130.png"),
            ("shield", "images/elements/victory_150.png"),
        ] {
            if self.has(path) {
                css.push_str(&format!(".dc-{name}{{clip-path:none;}}\n"));
            }
        }
        css
    }
}

#[derive(Clone, Copy)]
pub struct Assets(pub Signal<Rc<AssetDb>>);

/// The asset cache, subscribing the calling component to its arrival.
pub fn use_assets() -> Rc<AssetDb> {
    use_context::<Assets>().0.read().clone()
}

/// Provides [`use_assets`] to the app: fetches the manifest, then loads the fonts.
pub fn use_provide_assets() -> Signal<Rc<AssetDb>> {
    let mut db = use_context_provider(|| Assets(Signal::new(Rc::new(AssetDb::default())))).0;
    use_future(move || async move {
        let mut tries = 0;
        let loaded = loop {
            match imp::manifest().await {
                Ok(Some(m)) => break AssetDb::new(m),
                Ok(None) => break AssetDb::absent(),
                Err(()) if tries < 5 => {
                    tries += 1;
                    platform::sleep(2000).await;
                }
                Err(()) => break AssetDb::absent(),
            }
        };
        db.set(Rc::new(loaded));
    });
    let status = db.read().status;
    use_effect(use_reactive!(|status| {
        if status == Status::Loaded {
            spawn(async move {
                if imp::load_fonts().await {
                    let mut next = (*db.peek()).as_ref().clone();
                    next.fonts += 1;
                    db.set(Rc::new(next));
                }
            });
        }
    }));
    db
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod imp {
    use dominion_protocol::assets::Manifest;
    use gloo_net::http::Request;

    /// `Ok(None)`: the server has no cache.
    pub async fn manifest() -> Result<Option<Manifest>, ()> {
        let resp = Request::get("/assets/manifest.json")
            .send()
            .await
            .map_err(|_| ())?;
        match resp.status() {
            200 => Ok(resp.json::<Manifest>().await.ok()),
            204 | 404 => Ok(None),
            _ => Err(()),
        }
    }

    pub async fn load_fonts() -> bool {
        let js = r#"
            const faces = ["100px DomTitle", "100px DomBody", "bold 100px DomBody",
                           "italic 100px DomBody", "bold 100px DomNum", "100px DomCount"];
            await Promise.all(faces.map(f => document.fonts.load(f).catch(() => null)));
            return true;
        "#;
        dioxus::prelude::document::eval(js).await.is_ok()
    }
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
mod imp {
    use dominion_protocol::assets::Manifest;

    pub async fn manifest() -> Result<Option<Manifest>, ()> {
        Ok(None)
    }

    pub async fn load_fonts() -> bool {
        false
    }
}
