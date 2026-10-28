//! ARRANGE's edits of the sectors' layouts (#148, the owner's approved
//! mockup `docs/mockups/sectors-v2.html`): in ARRANGE each layout tab of
//! the LAYOUTS block gets ✎ (rename: an inline field, at most
//! [`TITLE_MAX`] characters, upper case; Enter or a tap elsewhere keeps
//! the text, Escape the old title) and ✕ (remove: the first tap turns the
//! tab into a red [`REMOVE_ASK`], a tap on it removes the layout, a tap
//! elsewhere drops the ask; a sector's last layout has no ✕), and each
//! sector's cell a `+` that adds an empty [`NEW_TITLE`] layout with an id
//! no revision ever used ([`new_id`]), up to [`MAX_LAYOUTS`]. A tap on a tab still
//! pages its sector. [`Edit`] is the whole edited set the mode shows; the
//! second tap on ARRANGE sends it as one change (`behave::arrange_drag`).
//! Nothing here touches Live, and nothing is sent before that tap.

use fohmixer_proto::sectors::{
    MAX_LAYOUTS, SectorId, SectorLayout, Sectors, SectorsDoc, Slot, TITLE_MAX, trim,
};

use crate::binding::sectors::{Picks, picked};

/// An added layout's title (then `NEW 2`, `NEW 3`, ...).
pub const NEW_TITLE: &str = "NEW";

/// The tab's text while its ✕ asks.
pub const REMOVE_ASK: &str = "REMOVE?";

/// ARRANGE's edit: every sector's layouts as the page shows them (their
/// slots, titles, the added and removed layouts), the rename in its field
/// (the layout and the text typed), the layout whose ✕ asks, the revision
/// the save will make and the layouts added in this edit so far (an added
/// layout's id names both, [`new_id`]: a device whose pick named a removed
/// layout must never show another).
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub sectors: Sectors,
    pub renaming: Option<(String, String)>,
    pub confirming: Option<String>,
    next_rev: u64,
    adds: usize,
}

/// What a tap on a layout's tab did in ARRANGE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabTap {
    /// It asked REMOVE?: the layout is gone.
    Removed,
    /// It pages its sector to the layout (any ask dropped).
    Page,
}

impl Edit {
    /// The edit of `doc`, nothing changed yet.
    pub fn of(doc: &SectorsDoc) -> Self {
        Self {
            sectors: doc.sectors.clone(),
            renaming: None,
            confirming: None,
            next_rev: doc.rev.saturating_add(1),
            adds: 0,
        }
    }

    /// Whether it changes `doc` (the trailing empty slots aside).
    pub fn changed(&self, doc: &SectorsDoc) -> bool {
        trimmed(&self.sectors) != doc.sectors
    }

    fn layout_mut(&mut self, id: &str) -> Option<&mut SectorLayout> {
        self.sectors
            .values_mut()
            .flatten()
            .find(|layout| layout.id == id)
    }

    fn sector_of(&self, id: &str) -> Option<SectorId> {
        self.sectors
            .iter()
            .find(|(_, layouts)| layouts.iter().any(|l| l.id == id))
            .map(|(sector, _)| *sector)
    }

    /// `layout`'s slots in the edit (none for a layout it does not hold).
    pub fn slots_of(&self, layout: &str) -> Option<&[Slot]> {
        self.sectors
            .values()
            .flatten()
            .find(|l| l.id == layout)
            .map(|l| l.strips.as_slice())
    }

    /// A drag gave `layout` these slots.
    pub fn moved(&mut self, layout: &str, slots: Vec<Slot>) {
        if let Some(target) = self.layout_mut(layout) {
            target.strips = slots;
        }
    }

    /// ✎ on `layout`: its field opens with its title (returned for the
    /// field; none for a layout the edit does not hold). Any ask drops.
    pub fn rename_start(&mut self, layout: &str) -> Option<String> {
        self.confirming = None;
        let title = self
            .sectors
            .values()
            .flatten()
            .find(|l| l.id == layout)?
            .title
            .clone();
        self.renaming = Some((layout.to_string(), title.clone()));
        Some(title)
    }

    /// The field holds `text`: what it shows ([`field_text`]), kept as the
    /// rename's text while one runs.
    pub fn typed(&mut self, text: &str) -> String {
        let shown = field_text(text);
        if let Some((_, draft)) = &mut self.renaming {
            draft.clone_from(&shown);
        }
        shown
    }

    /// The field closes: with `keep` (Enter, a tap elsewhere) its text
    /// becomes the title unless it is blank; without (Escape) the title
    /// stays.
    pub fn rename_end(&mut self, keep: bool) {
        let Some((layout, draft)) = self.renaming.take() else {
            return;
        };
        if let Some(title) = committed(&draft).filter(|_| keep)
            && let Some(target) = self.layout_mut(&layout)
        {
            target.title = title;
        }
    }

    /// `layout`'s field closes (its Enter, Escape or blur) as
    /// [`Edit::rename_end`], only while it is the one renaming: the blur of
    /// a field another ✎ took over ends nothing.
    pub fn rename_end_of(&mut self, layout: &str, keep: bool) {
        if self.renaming.as_ref().is_some_and(|(id, _)| id == layout) {
            self.rename_end(keep);
        }
    }

    /// Whether `sector` can take one more layout (the `+` shows).
    pub fn can_add(&self, sector: SectorId) -> bool {
        self.sectors
            .get(&sector)
            .is_some_and(|layouts| layouts.len() < MAX_LAYOUTS)
    }

    /// `+` in `sector`: an empty layout titled [`new_title`] with a
    /// [`new_id`], last; its id (none at [`MAX_LAYOUTS`] or for a sector
    /// the document does not name). Any ask drops.
    pub fn add(&mut self, sector: SectorId) -> Option<String> {
        self.confirming = None;
        if !self.can_add(sector) {
            return None;
        }
        let layouts = self.sectors.get_mut(&sector)?;
        self.adds += 1;
        let id = new_id(sector, self.next_rev, self.adds);
        let title = new_title(layouts);
        layouts.push(SectorLayout {
            id: id.clone(),
            title,
            strips: Vec::new(),
        });
        Some(id)
    }

    /// Whether `sector`'s layouts can lose one (its last has no ✕).
    pub fn removable(&self, sector: SectorId) -> bool {
        self.sectors
            .get(&sector)
            .is_some_and(|layouts| layouts.len() > 1)
    }

    /// ✕ on `layout`: its tab asks REMOVE?, or stops asking when it did.
    /// Nothing for a sector's last layout.
    pub fn ask_remove(&mut self, layout: &str) {
        let asked = self.confirming.as_deref() == Some(layout);
        let can = self.sector_of(layout).is_some_and(|s| self.removable(s));
        self.confirming = (can && !asked).then(|| layout.to_string());
    }

    /// A tap on `layout`'s tab: removes it while it asks, else pages to it.
    pub fn tab_tapped(&mut self, layout: &str) -> TabTap {
        let asked = self.confirming.take().as_deref() == Some(layout);
        let sector = self.sector_of(layout).filter(|s| self.removable(*s));
        match (asked, sector) {
            (true, Some(sector)) => {
                if let Some(layouts) = self.sectors.get_mut(&sector) {
                    layouts.retain(|l| l.id != layout);
                }
                TabTap::Removed
            }
            _ => TabTap::Page,
        }
    }

    /// A down anywhere on the page (before its own handler): one off the
    /// REMOVE? tab and every ✕ (`on_ask`) drops the ask, one off the field
    /// (`on_field`) ends the rename keeping its text.
    pub fn outside_down(&mut self, on_ask: bool, on_field: bool) {
        if !on_ask {
            self.confirming = None;
        }
        if !on_field {
            self.rename_end(true);
        }
    }
}

/// `sectors` with every layout's trailing empty slots dropped.
fn trimmed(sectors: &Sectors) -> Sectors {
    sectors
        .iter()
        .map(|(sector, layouts)| {
            let layouts = layouts
                .iter()
                .map(|l| SectorLayout {
                    id: l.id.clone(),
                    title: l.title.clone(),
                    strips: trim(l.strips.clone()),
                })
                .collect();
            (*sector, layouts)
        })
        .collect()
}

/// What the rename field shows of `text`: upper case, no control
/// character, at most [`TITLE_MAX`] characters.
pub fn field_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .flat_map(char::to_uppercase)
        .take(TITLE_MAX)
        .collect()
}

/// The title a field's `text` sets: trimmed, none when blank.
pub fn committed(text: &str) -> Option<String> {
    Some(text.trim())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// An added layout's title: [`NEW_TITLE`] when free in `layouts`, else
/// `NEW 2`, `NEW 3`, ... (the first free; at most [`MAX_LAYOUTS`] layouts
/// leave one free).
pub fn new_title(layouts: &[SectorLayout]) -> String {
    let free = |title: &str| !layouts.iter().any(|l| l.title == title);
    std::iter::once(NEW_TITLE.to_string())
        .chain((2..=MAX_LAYOUTS + 1).map(|n| format!("{NEW_TITLE} {n}")))
        .find(|title| free(title.as_str()))
        .unwrap_or_else(|| NEW_TITLE.to_string())
}

/// An added layout's id in `sector`: `<sector>-r<rev>-<n>`, `rev` the
/// revision its save makes and `n` the edit's `n`-th add. Revisions only
/// grow (an undo and a hand edit make new ones too) and only one save
/// makes each, so no id is ever used again, also after its layout was
/// removed or undone (#148 review).
pub fn new_id(sector: SectorId, rev: u64, n: usize) -> String {
    format!("{}-r{rev}-{n}", sector.as_str())
}

/// One tab of a sector's cell in the LAYOUTS block: its layout and title,
/// whether the device shows it, whether its ✕ asks, whether its field is
/// open, whether the sector can lose a layout (its ✕ shows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRow {
    pub id: String,
    pub title: String,
    pub selected: bool,
    pub asks: bool,
    pub renaming: bool,
    pub removable: bool,
}

/// `sector`'s tabs as `doc` (the document shown: the edit while ARRANGE is
/// on) and the device's `picks` have them; the ask and the field from
/// `edit` (none outside ARRANGE).
pub fn tab_rows(
    doc: &SectorsDoc,
    picks: &Picks,
    sector: SectorId,
    edit: Option<&Edit>,
) -> Vec<TabRow> {
    let layouts = doc.layouts(sector);
    let selected = picked(doc, picks, sector).map(|l| l.id.as_str());
    let asking = edit.and_then(|e| e.confirming.as_deref());
    let renaming = edit.and_then(|e| e.renaming.as_ref().map(|(id, _)| id.as_str()));
    layouts
        .iter()
        .map(|l| TabRow {
            id: l.id.clone(),
            title: l.title.clone(),
            selected: selected == Some(l.id.as_str()),
            asks: asking == Some(l.id.as_str()),
            renaming: renaming == Some(l.id.as_str()),
            removable: layouts.len() > 1,
        })
        .collect()
}

#[cfg(test)]
mod tests;
