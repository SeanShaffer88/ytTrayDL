//! Settings-only pagination. Controls are rendered once, on their visible page.
use std::ops::Range;

pub fn pages(heights: &[f32], budget: f32) -> Vec<Range<usize>> {
    let budget = budget.max(1.0);
    let mut result = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    for (index, height) in heights.iter().enumerate() {
        let height = height.max(1.0);
        if index > start && used + height > budget {
            result.push(start..index);
            start = index;
            used = 0.0;
        }
        used += height;
    }
    result.push(start..heights.len());
    result
}

pub fn collection_range(index: &mut usize, count: usize, capacity: usize) -> Range<usize> {
    let capacity = capacity.max(1);
    let last = count.saturating_sub(1) / capacity;
    *index = (*index).min(last);
    let start = (*index * capacity).min(count);
    start..(start + capacity).min(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_preserve_every_atomic_row_at_small_and_large_sizes() {
        let heights = [52.0, 52.0, 28.0, 52.0, 28.0, 72.0];
        for budget in [72.0, 120.0, 230.0, 800.0] {
            let plan = pages(&heights, budget);
            assert_eq!(
                plan.iter().flat_map(|r| r.clone()).collect::<Vec<_>>(),
                (0..6).collect::<Vec<_>>()
            );
            assert!(plan
                .iter()
                .all(|r| heights[r.clone()].iter().sum::<f32>() <= budget));
        }
    }
    #[test]
    fn collection_pagination_clamps_after_delete_and_resize_without_missing_items() {
        for count in [0usize, 1, 2, 99, 10000] {
            for capacity in [1, 2, 7] {
                let mut seen = Vec::new();
                for page in 0..count.max(1).div_ceil(capacity) {
                    let mut current = page;
                    seen.extend(collection_range(&mut current, count, capacity));
                }
                assert_eq!(seen, (0..count).collect::<Vec<_>>());
                let mut stale = usize::MAX;
                let range = collection_range(&mut stale, count, capacity);
                assert!(range.end <= count);
            }
        }
    }
}
