//! The `Updated to <App> X.Y.Z` notice (ADR 0012), at the first start of a version an
//! update applied: a banner at the top of the first window's content, over it, until it's
//! dismissed or [`SHOWN_FOR`] has passed. It never blocks the window or takes the keys.
//!
//! Each app puts its windows' content in a [`host`] and gives the notice, if it has one,
//! to [`queue`] before it runs (`suite_update::notice` says whether there is one; this
//! crate doesn't depend on `suite-update`). The first host to show claims its window; a
//! host that later replaces the content of that window ("Opening…", then the document)
//! shows the notice again until it's gone. When it first shows, the log gets
//! `update: Updated to <App> X.Y.Z` on stderr, which the update checks read.

use gtk::prelude::*;
use gtk::{glib, Align};
use std::cell::RefCell;
use std::time::Duration;

/// How long the notice shows unless it's dismissed sooner.
pub const SHOWN_FOR: Duration = Duration::from_secs(20);

/// The notice waiting for a window, or showing in one.
struct Notice {
    text: String,
    /// The window it showed in first: only that window shows it.
    window: Option<glib::WeakRef<gtk::Window>>,
    /// The banner showing now.
    banner: Option<glib::WeakRef<gtk::Revealer>>,
    /// Dismissed, or its time is up: it never shows again.
    gone: bool,
}

thread_local! {
    static NOTICE: RefCell<Option<Notice>> = const { RefCell::new(None) };
}

/// Show `text` (if any) in the first window whose content is in a [`host`]. Call on the
/// main thread before the app runs.
pub fn queue(text: Option<String>) {
    NOTICE.set(text.map(|text| Notice {
        text,
        window: None,
        banner: None,
        gone: false,
    }));
}

/// `content` as a window's child, in an overlay the notice can show over.
pub fn host(content: &impl IsA<gtk::Widget>) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(content));
    overlay.connect_map(show_in);
    overlay
}

/// The notice in `host`, if it's still to show and `host` is in its window.
fn show_in(host: &gtk::Overlay) {
    let Some(window) = host.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let text = NOTICE.with_borrow_mut(|notice| {
        let notice = notice.as_mut().filter(|n| !n.gone)?;
        // Mapped again (the window shown again): its banner is still there.
        let showing = notice.banner.as_ref().and_then(|b| b.upgrade());
        if showing.is_some_and(|b| b.parent().as_ref() == Some(host.upcast_ref())) {
            return None;
        }
        match notice.window.as_ref().map(|w| w.upgrade()) {
            Some(Some(w)) if w == window => {}
            Some(_) => return None,
            None => {
                notice.window = Some(window.downgrade());
                eprintln!("update: {}", notice.text);
                glib::timeout_add_local_once(SHOWN_FOR, dismiss);
            }
        }
        Some(notice.text.clone())
    });
    let Some(text) = text else {
        return;
    };
    let banner = banner(&text);
    host.add_overlay(&banner);
    banner.set_reveal_child(true);
    NOTICE.with_borrow_mut(|notice| {
        if let Some(notice) = notice {
            notice.banner = Some(banner.downgrade());
        }
    });
}

/// The notice goes, for good.
fn dismiss() {
    let banner = NOTICE.with_borrow_mut(|notice| {
        let notice = notice.as_mut()?;
        notice.gone = true;
        notice.banner.take()?.upgrade()
    });
    if let Some(banner) = banner {
        banner.connect_child_revealed_notify(|banner| {
            if let Some(host) = banner.parent().and_downcast::<gtk::Overlay>() {
                host.remove_overlay(banner);
            }
        });
        banner.set_reveal_child(false);
    }
}

/// The banner: the text and a button that dismisses it, sliding down from the top edge,
/// centred. Nothing in it takes the keys unless the user tabs or clicks there.
fn banner(text: &str) -> gtk::Revealer {
    let label = gtk::Label::new(Some(text));
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("flat");
    close.set_tooltip_text(Some("Dismiss"));
    close.update_property(&[gtk::accessible::Property::Label("Dismiss")]);
    close.set_focus_on_click(false);
    close.connect_clicked(|_| dismiss());
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("app-notification");
    row.append(&label);
    row.append(&close);
    gtk::Revealer::builder()
        .child(&row)
        .halign(Align::Center)
        .valign(Align::Start)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build()
}
