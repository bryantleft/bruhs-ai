//! Render-ready assistant content, kept between frames.
//!
//! Splitting a message into prose / math / mermaid blocks parses its
//! markdown and builds SVG images. A frame used to redo that for every
//! message, so each streamed token cost O(transcript). [`ContentBlockCache`]
//! keeps the prepared blocks per message and redoes the work only for a
//! message whose text (or streaming state) changed.

use super::{
    desktop_assistant_content_blocks_while, DesktopAssistantContentBlock, DesktopMathRender,
    DesktopMermaidRender,
};
use gpui::{Image, ImageFormat, SharedString};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

/// An SVG rendered to an image once, or the error that replaced it.
#[derive(Clone)]
pub(super) enum PreparedSvg {
    Image(Arc<Image>),
    Error(SharedString),
}

impl PreparedSvg {
    fn from_svg(svg: String) -> Self {
        Self::Image(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            svg.into_bytes(),
        )))
    }
}

/// One block of an assistant message, ready to turn into elements.
#[derive(Clone)]
pub(super) enum PreparedBlock {
    Markdown(SharedString),
    Math {
        display: bool,
        render: PreparedSvg,
        source: SharedString,
    },
    Mermaid {
        render: PreparedSvg,
        source: SharedString,
    },
}

fn prepare(block: DesktopAssistantContentBlock) -> Option<PreparedBlock> {
    Some(match block {
        DesktopAssistantContentBlock::Markdown(markdown) => {
            if markdown.trim().is_empty() {
                return None;
            }
            PreparedBlock::Markdown(markdown.into())
        }
        DesktopAssistantContentBlock::Math(block) => PreparedBlock::Math {
            display: block.display,
            render: match block.render {
                DesktopMathRender::Svg(svg) => PreparedSvg::from_svg(svg),
                DesktopMathRender::Error(error) => PreparedSvg::Error(error.into()),
            },
            source: block.source.into(),
        },
        DesktopAssistantContentBlock::Mermaid(block) => PreparedBlock::Mermaid {
            render: match block.render {
                DesktopMermaidRender::Svg(svg) => PreparedSvg::from_svg(svg),
                DesktopMermaidRender::Error(error) => PreparedSvg::Error(error.into()),
            },
            source: block.source.into(),
        },
    })
}

/// A message's blocks with their position among all of its blocks (empty
/// prose blocks are skipped but keep their index, so element ids are the
/// same as when every block was rendered).
pub(super) type PreparedBlocks = Rc<[(usize, PreparedBlock)]>;

/// What a message's prepared blocks were built from.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ContentKey {
    len: usize,
    hash: u64,
    streaming: bool,
}

impl ContentKey {
    fn of(content: &str, streaming: bool) -> Self {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Self {
            len: content.len(),
            hash: hasher.finish(),
            streaming,
        }
    }
}

/// Prepared blocks per message index.
#[derive(Default)]
pub(super) struct ContentBlockCache {
    entries: RefCell<HashMap<usize, (ContentKey, PreparedBlocks)>>,
    #[cfg(test)]
    pub(super) builds: std::cell::Cell<usize>,
}

impl ContentBlockCache {
    /// The blocks of message `index` with text `content`, built only when
    /// they are not cached for exactly this text and streaming state.
    pub(super) fn blocks(&self, index: usize, content: &str, streaming: bool) -> PreparedBlocks {
        let key = ContentKey::of(content, streaming);
        if let Some((cached, blocks)) = self.entries.borrow().get(&index) {
            if *cached == key {
                return Rc::clone(blocks);
            }
        }
        #[cfg(test)]
        self.builds.set(self.builds.get() + 1);
        let blocks: PreparedBlocks = desktop_assistant_content_blocks_while(content, streaming)
            .into_iter()
            .enumerate()
            .filter_map(|(index, block)| prepare(block).map(|block| (index, block)))
            .collect();
        self.entries
            .borrow_mut()
            .insert(index, (key, Rc::clone(&blocks)));
        blocks
    }

    /// Forget messages past `len` (the transcript shrank).
    pub(super) fn truncate(&self, len: usize) {
        self.entries.borrow_mut().retain(|index, _| *index < len);
    }

    pub(super) fn clear(&self) {
        self.entries.borrow_mut().clear();
    }
}

/// A map that keeps at most `capacity` entries, evicting the least recently
/// used. Small capacities only: lookups touch the recency queue linearly.
pub(super) struct BoundedCache<K, V> {
    capacity: usize,
    map: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Clone + Eq + Hash, V: Clone> BoundedCache<K, V> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub(super) fn get(&mut self, key: &K) -> Option<V> {
        let value = self.map.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    pub(super) fn insert(&mut self, key: K, value: V) {
        if self.map.insert(key.clone(), value).is_some() {
            self.touch(&key);
            return;
        }
        self.order.push_back(key);
        while self.map.len() > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.map.remove(&oldest);
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.map.len()
    }

    fn touch(&mut self, key: &K) {
        if let Some(position) = self.order.iter().position(|queued| queued == key) {
            if let Some(key) = self.order.remove(position) {
                self.order.push_back(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedCache, ContentBlockCache, PreparedBlock};

    #[test]
    fn bounded_cache_evicts_the_least_recently_used_entry() {
        let mut cache = BoundedCache::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        assert_eq!(cache.get(&"a"), Some(1));
        cache.insert("c", 3);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"b"), None, "b was least recently used");
        assert_eq!(cache.get(&"a"), Some(1));
        assert_eq!(cache.get(&"c"), Some(3));
    }

    #[test]
    fn bounded_cache_never_grows_past_capacity() {
        let mut cache = BoundedCache::new(8);
        for n in 0..1_000 {
            cache.insert(n, n);
        }
        assert_eq!(cache.len(), 8);
    }

    #[test]
    fn unchanged_messages_are_not_rebuilt() {
        let cache = ContentBlockCache::default();
        let first = cache.blocks(0, "Hello **world**", false);
        let again = cache.blocks(0, "Hello **world**", false);
        assert!(std::rc::Rc::ptr_eq(&first, &again));
        assert_eq!(cache.builds.get(), 1);

        cache.blocks(0, "Hello **world**!", false);
        assert_eq!(cache.builds.get(), 2, "new text rebuilds");
        cache.blocks(0, "Hello **world**!", true);
        assert_eq!(cache.builds.get(), 3, "a streaming change rebuilds");
        assert!(matches!(
            cache.blocks(1, "", false).first(),
            None | Some((0, PreparedBlock::Markdown(_)))
        ));
    }

    #[test]
    fn truncate_forgets_messages_past_the_end() {
        let cache = ContentBlockCache::default();
        cache.blocks(0, "zero", false);
        cache.blocks(1, "one", false);
        cache.truncate(1);
        cache.blocks(0, "zero", false);
        assert_eq!(cache.builds.get(), 2);
        cache.blocks(1, "one", false);
        assert_eq!(cache.builds.get(), 3);
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::super::{desktop_assistant_content_blocks_while, DesktopAssistantContentBlock};

    const OPEN: &str = "Diagram:\n\n```mermaid\ngraph TD\nA-->B";

    fn has_mermaid(blocks: &[DesktopAssistantContentBlock]) -> bool {
        blocks
            .iter()
            .any(|block| matches!(block, DesktopAssistantContentBlock::Mermaid(_)))
    }

    #[test]
    fn an_unclosed_mermaid_fence_is_not_rendered_while_streaming() {
        let blocks = desktop_assistant_content_blocks_while(OPEN, true);
        assert!(!has_mermaid(&blocks), "{blocks:?}");
        assert!(blocks.iter().any(|block| matches!(
            block,
            DesktopAssistantContentBlock::Markdown(text) if text.contains("A-->B")
        )));
    }

    #[test]
    fn a_closed_mermaid_fence_renders_while_streaming() {
        let closed = format!("{OPEN}\n```\n\nMore text");
        assert!(has_mermaid(&desktop_assistant_content_blocks_while(
            &closed, true
        )));
    }

    #[test]
    fn a_finished_message_renders_its_unclosed_fence_as_before() {
        assert!(has_mermaid(&desktop_assistant_content_blocks_while(
            OPEN, false
        )));
    }
}
