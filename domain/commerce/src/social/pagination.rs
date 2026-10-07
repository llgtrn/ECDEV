//! Bounded walking of opaque page cursors (Algolia page numbers, Bluesky cursors, JSON Feed
//! next_url). A walk stops at the first of: the source signalling no further page, an empty
//! page, a cursor already followed (a stalled or cycling source), or the page limit. The stop
//! reason is evidence of how complete the retrieved sample is; none of them means "all posts".

use std::collections::BTreeSet;

pub const MAX_SOCIAL_PAGES: u64 = 5;

#[derive(Debug, Default)]
pub struct PageWalk {
    seen: BTreeSet<String>,
    pages: u64,
    max: u64,
}

impl PageWalk {
    pub fn new(max_pages: u64) -> Self {
        Self {
            max: max_pages.clamp(1, MAX_SOCIAL_PAGES),
            ..Default::default()
        }
    }
    pub fn pages(&self) -> u64 {
        self.pages
    }
    /// After a page with `posts` posts whose source offers `next`: Ok(cursor) to fetch next,
    /// or Err(stop reason).
    pub fn after_page(&mut self, posts: usize, next: Option<&str>) -> Result<String, &'static str> {
        self.pages += 1;
        let next = next.map(str::trim).filter(|c| !c.is_empty());
        if posts == 0 {
            return Err("EMPTY_PAGE");
        }
        let Some(next) = next else {
            return Err("END_OF_RESULTS");
        };
        if self.pages >= self.max {
            return Err("PAGE_LIMIT");
        }
        if !self.seen.insert(next.to_string()) {
            return Err("REPEATED_CURSOR");
        }
        Ok(next.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_stop_for_a_stated_reason() {
        let mut w = PageWalk::new(5);
        assert_eq!(w.after_page(50, Some("c1")), Ok("c1".into()));
        assert_eq!(w.after_page(50, Some("c2")), Ok("c2".into()));
        assert_eq!(w.after_page(50, Some("c1")), Err("REPEATED_CURSOR"));
        let mut w = PageWalk::new(5);
        assert_eq!(w.after_page(50, None), Err("END_OF_RESULTS"));
        assert_eq!(
            PageWalk::new(5).after_page(50, Some("  ")),
            Err("END_OF_RESULTS")
        );
        assert_eq!(
            PageWalk::new(5).after_page(0, Some("c1")),
            Err("EMPTY_PAGE")
        );
        let mut w = PageWalk::new(2);
        assert!(w.after_page(10, Some("a")).is_ok());
        assert_eq!(w.after_page(10, Some("b")), Err("PAGE_LIMIT"));
        assert_eq!(w.pages(), 2);
        // One page is the default and the floor; the ceiling is MAX_SOCIAL_PAGES.
        assert_eq!(
            PageWalk::new(0).after_page(10, Some("a")),
            Err("PAGE_LIMIT")
        );
        assert_eq!(PageWalk::new(99).max, MAX_SOCIAL_PAGES);
    }
}
