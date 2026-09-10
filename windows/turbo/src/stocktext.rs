//! Extra text banks and persistent collision displacement.
//!
//! The native owner has room for four pointers. Overflow lives separately,
//! while each bank is visited through the same create, update, and draw paths.
//! Zero marks a free slot; the client clears slots when it releases a line.

/// Collision displacement retained while a native text line is alive.
///
/// The projected position still follows the unit and the text's animation.
/// Only the collision adjustment carries across frames, so crowded text does
/// not repeatedly jump between equally suitable gaps around the unit.
#[derive(Default)]
pub struct Placement {
    offset: [f32; 2],
    placed: bool,
}

impl Placement {
    /// Start the next search at the line's displaced position.
    pub fn seed(&self, rect: &mut [f32; 4]) {
        rect[0] += self.offset[1];
        rect[2] += self.offset[1];
        rect[1] += self.offset[0];
        rect[3] += self.offset[0];
    }

    /// Keep the displacement from the current projection, not its old position.
    pub fn remember(&mut self, projected: [f32; 4], placed: [f32; 4]) {
        self.offset = [placed[1] - projected[1], placed[2] - projected[2]];
        self.placed = true;
    }

    /// Keep a previous placement when it still fits the screen and its neighbors.
    pub fn fits(&self, rect: [f32; 4], screen: [f32; 2], obstacles: &[[f32; 4]]) -> bool {
        // Adjacent boxes differ by a few ulps after offset addition. Those
        // rounding differences must not trigger a new search across the screen.
        let epsilon = screen.map(|extent| extent * (8.0 * f32::EPSILON));
        self.placed
            && rect[1] >= 0.0
            && rect[2] >= 0.0
            && rect[3] <= screen[0]
            && rect[0] <= screen[1]
            && obstacles.iter().all(|other| {
                rect[3].min(other[3]) - rect[1].max(other[1]) <= epsilon[0]
                    || rect[0].min(other[0]) - rect[2].max(other[2]) <= epsilon[1]
            })
    }
}

/// Extra banks belonging to one native text owner.
#[derive(Default)]
pub struct Banks {
    slots: Vec<[usize; 4]>,
}

impl Banks {
    /// Whether any bank still owns a line.
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Offer creation a bank with a free slot, growing only when all are full.
    pub fn create(&mut self, create: impl FnOnce([usize; 4]) -> [usize; 4]) {
        let index = self
            .slots
            .iter()
            .position(|slots| slots.contains(&0))
            .unwrap_or(self.slots.len());
        if index == self.slots.len() {
            self.slots.push([0; 4]);
        }
        self.slots[index] = create(self.slots[index]);
        self.slots.retain(|slots| *slots != [0; 4]);
    }

    /// Visit every bank, retaining only those still owning live lines.
    pub fn visit(&mut self, mut visit: impl FnMut([usize; 4]) -> [usize; 4]) {
        self.slots.retain_mut(|slots| {
            *slots = visit(*slots);
            *slots != [0; 4]
        });
    }

    /// Return every remaining line exactly once for owner destruction.
    pub fn into_lines(self) -> impl Iterator<Item = usize> {
        self.slots.into_iter().flatten().filter(|&line| line != 0)
    }

    /// Keep banks created while an earlier visit was outside the registry.
    pub fn append(&mut self, other: &mut Self) {
        self.slots.append(&mut other.slots);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_follows_projection_without_restarting_the_collision_search() {
        let mut placement = Placement::default();
        let projected = [0.5, 0.25, 0.25, 0.5];
        let displaced = [0.75, 0.5, 0.5, 0.75];
        placement.remember(projected, displaced);
        // Both camera motion and a shrinking glyph box remain in the seed.
        let next = [0.5625, 0.25, 0.375, 0.4375];
        let mut seeded = next;
        placement.seed(&mut seeded);
        assert_eq!(seeded, [0.8125, 0.5, 0.625, 0.6875]);
        placement.remember(next, seeded);
        let mut repeated = next;
        placement.seed(&mut repeated);
        assert_eq!(repeated, seeded);
    }

    #[test]
    fn a_new_line_starts_without_the_previous_lines_displacement() {
        let mut rect = [0.5, 0.25, 0.25, 0.5];
        Placement::default().seed(&mut rect);
        assert_eq!(rect, [0.5, 0.25, 0.25, 0.5]);
    }

    #[test]
    fn adjacent_box_rounding_keeps_position_but_real_overlap_requires_search() {
        let rect = [0.5, 0.25, 0.25, 0.5];
        let mut placement = Placement::default();
        assert!(!placement.fits(rect, [1.0, 1.0], &[]));
        placement.remember(rect, rect);
        assert!(placement.fits(rect, [1.0, 1.0], &[]));
        let touching = [0.5, 0.5 - f32::EPSILON, 0.25, 0.75];
        assert!(placement.fits(rect, [1.0, 1.0], &[touching]));
        let overlapping = [0.5, 0.4375, 0.25, 0.75];
        assert!(!placement.fits(rect, [1.0, 1.0], &[overlapping]));
        assert!(!placement.fits([1.125, 0.25, 0.875, 0.5], [1.0, 1.0], &[]));
    }

    fn insert(banks: &mut Banks, line: usize) {
        banks.create(|mut slots| {
            let free = slots.iter_mut().find(|slot| **slot == 0).unwrap();
            *free = line;
            slots
        });
    }

    #[test]
    fn burst_keeps_every_line_and_visits_it_once() {
        let mut banks = Banks::default();
        for line in 1..=257 {
            insert(&mut banks, line);
        }
        let mut visited = Vec::new();
        banks.visit(|slots| {
            visited.extend(slots.into_iter().filter(|&line| line != 0));
            slots
        });
        assert_eq!(visited, (1..=257).collect::<Vec<_>>());
        assert_eq!(banks.into_lines().collect::<Vec<_>>(), visited);
    }

    #[test]
    fn expired_lines_are_not_destroyed_twice_and_holes_are_reused() {
        let mut banks = Banks::default();
        for line in 1..=9 {
            insert(&mut banks, line);
        }
        banks.visit(|slots| slots.map(|line| if line % 2 == 0 { line } else { 0 }));
        insert(&mut banks, 10);
        assert_eq!(banks.slots.len(), 2);
        assert_eq!(banks.into_lines().collect::<Vec<_>>(), [10, 2, 4, 6, 8]);
    }

    #[test]
    fn failed_creation_and_complete_expiry_leave_no_banks() {
        let mut banks = Banks::default();
        banks.create(|slots| slots);
        assert!(banks.is_empty());
        insert(&mut banks, 1);
        banks.visit(|_| [0; 4]);
        assert!(banks.is_empty());
    }

    #[test]
    fn banks_returned_from_a_visit_preserve_nested_creations() {
        let mut banks = Banks::default();
        let mut nested = Banks::default();
        insert(&mut banks, 1);
        insert(&mut nested, 2);
        banks.append(&mut nested);
        assert!(nested.is_empty());
        assert_eq!(banks.into_lines().collect::<Vec<_>>(), [1, 2]);
    }
}
