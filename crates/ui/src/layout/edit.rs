//! Pure edit operations (§6): never overlap, never out of bounds — proptested.
//! The interactive edit *mode* arrives in arc 4; the ops land with the engine.

use super::grid::GridSpec;
use super::page::{Page, Placement};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    OutOfBounds,
    Overlap,
    NoSuchPlacement,
    NoRoom,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::OutOfBounds => f.write_str("outside the grid"),
            EditError::Overlap => f.write_str("would overlap another tile"),
            EditError::NoSuchPlacement => f.write_str("no such placement"),
            EditError::NoRoom => f.write_str("no free slot fits"),
        }
    }
}

fn validate(spec: &GridSpec, page: &Page, changed: usize) -> Result<(), EditError> {
    let p = &page.place[changed];
    if !p.in_bounds(spec.columns, spec.rows) {
        return Err(EditError::OutOfBounds);
    }
    for (i, other) in page.place.iter().enumerate() {
        if i != changed && p.overlaps(other) {
            return Err(EditError::Overlap);
        }
    }
    Ok(())
}

pub fn move_by(
    spec: &GridSpec,
    page: &Page,
    idx: usize,
    dx: i8,
    dy: i8,
) -> Result<Page, EditError> {
    let mut next = page.clone();
    let p = next.place.get_mut(idx).ok_or(EditError::NoSuchPlacement)?;
    let nx = i16::from(p.at.0) + i16::from(dx);
    let ny = i16::from(p.at.1) + i16::from(dy);
    if nx < 0 || ny < 0 {
        return Err(EditError::OutOfBounds);
    }
    p.at = (nx as u8, ny as u8);
    validate(spec, &next, idx)?;
    Ok(next)
}

pub fn resize_by(
    spec: &GridSpec,
    page: &Page,
    idx: usize,
    dw: i8,
    dh: i8,
) -> Result<Page, EditError> {
    let mut next = page.clone();
    let p = next.place.get_mut(idx).ok_or(EditError::NoSuchPlacement)?;
    let nw = i16::from(p.size.0) + i16::from(dw);
    let nh = i16::from(p.size.1) + i16::from(dh);
    if nw < 1 || nh < 1 {
        return Err(EditError::OutOfBounds);
    }
    p.size = (nw as u8, nh as u8);
    validate(spec, &next, idx)?;
    Ok(next)
}

pub fn swap(_spec: &GridSpec, page: &Page, a: usize, b: usize) -> Result<Page, EditError> {
    if a >= page.place.len() || b >= page.place.len() {
        return Err(EditError::NoSuchPlacement);
    }
    let mut next = page.clone();
    let (pa_at, pa_size) = (next.place[a].at, next.place[a].size);
    let (pb_at, pb_size) = (next.place[b].at, next.place[b].size);
    next.place[a].at = pb_at;
    next.place[a].size = pb_size;
    next.place[b].at = pa_at;
    next.place[b].size = pa_size;
    validate(_spec, &next, a)?;
    validate(_spec, &next, b)?;
    Ok(next)
}

pub fn remove(page: &Page, idx: usize) -> Result<Page, EditError> {
    if idx >= page.place.len() {
        return Err(EditError::NoSuchPlacement);
    }
    let mut next = page.clone();
    next.place.remove(idx);
    Ok(next)
}

/// Place at the first free slot scanning rows then columns (§10 picker).
pub fn insert_first_fit(
    spec: &GridSpec,
    page: &Page,
    mut placement: Placement,
) -> Result<Page, EditError> {
    for y in 0..spec.rows {
        for x in 0..spec.columns {
            placement.at = (x, y);
            if !placement.in_bounds(spec.columns, spec.rows) {
                continue;
            }
            if page.place.iter().all(|other| !placement.overlaps(other)) {
                let mut next = page.clone();
                next.place.push(placement);
                return Ok(next);
            }
        }
    }
    Err(EditError::NoRoom)
}
