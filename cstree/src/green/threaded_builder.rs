use dashmap::DashMap;
use fxhash::{FxBuildHasher, FxHasher32};
use std::hash::{Hash, Hasher};
use text_size::TextSize;

use super::{node::GreenNodeHead, token::GreenTokenData};
use crate::interning::{new_threaded_interner, MultiThreadedTokenInterner};
use crate::utility_types::MaybeOwnedRef;
use crate::{
    green::{GreenElement, GreenNode, GreenToken},
    interning::{Interner, TokenKey},
    util::NodeOrToken,
    RawSyntaxKind, Syntax,
};

/// If `node.children() <= CHILDREN_CACHE_THRESHOLD`, we will not create
/// a new [`GreenNode`], but instead lookup in the cache if this node is
/// already present. If so we use the one in the cache, otherwise we insert
/// this node into the cache.
const CHILDREN_CACHE_THRESHOLD: usize = 3;

/// A `NodeCache` deduplicates identical tokens and small nodes during tree construction.
/// You can re-use the same cache for multiple similar trees with [`ThreadedGreenNodeBuilder::with_cache`].
#[derive(Debug)]
pub struct ThreadedNodeCache<'i, I = MultiThreadedTokenInterner> {
    nodes: DashMap<GreenNodeHead, GreenNode, FxBuildHasher>,
    tokens: DashMap<GreenTokenData, GreenToken, FxBuildHasher>,
    interner: MaybeOwnedRef<'i, I>,
}

impl ThreadedNodeCache<'static> {
    /// Constructs a new, empty cache.
    ///
    /// By default, this will also create a default interner to deduplicate source text (strings) across
    /// tokens. To re-use an existing interner, see [`with_interner`](ThreadedNodeCache::with_interner).
    /// # Examples
    /// ```
    /// # use cstree::testing::*;
    /// use cstree::build::NodeCache;
    ///
    /// // Build a tree
    /// let mut cache = NodeCache::new();
    /// let mut builder: GreenNodeBuilder<MySyntax> = GreenNodeBuilder::with_cache(&mut cache);
    /// # builder.start_node(Root);
    /// # builder.token(Int, "42");
    /// # builder.finish_node();
    /// parse(&mut builder, "42");
    /// let (tree, _) = builder.finish();
    ///
    /// // Check it out!
    /// assert_eq!(tree.kind(), MySyntax::into_raw(Root));
    /// let int = tree.children().next().unwrap();
    /// assert_eq!(int.kind(), MySyntax::into_raw(Int));
    /// ```
    pub fn new() -> Self {
        Self {
            nodes: DashMap::default(),
            tokens: DashMap::default(),
            interner: MaybeOwnedRef::Owned(new_threaded_interner()),
        }
    }
}

impl Default for ThreadedNodeCache<'static> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'i, I> ThreadedNodeCache<'i, I>
where
    for<'a> &'a I: Interner<TokenKey>,
{
    /// Constructs a new, empty cache that will use the given interner to deduplicate source text
    /// (strings) across tokens.
    /// # Examples
    /// ```
    /// # use cstree::testing::*;
    /// # use cstree::interning::*;
    /// use cstree::build::NodeCache;
    ///
    /// // Create the builder from a custom interner
    /// let mut interner = new_interner();
    /// let mut cache = NodeCache::with_interner(&mut interner);
    /// let mut builder: GreenNodeBuilder<MySyntax, TokenInterner> =
    ///     GreenNodeBuilder::with_cache(&mut cache);
    ///
    /// // Construct the tree
    /// # builder.start_node(Root);
    /// # builder.token(Int, "42");
    /// # builder.finish_node();
    /// parse(&mut builder, "42");
    /// let (tree, _) = builder.finish();
    ///
    /// // Use the tree
    /// assert_eq!(tree.kind(), MySyntax::into_raw(Root));
    /// let int = tree.children().next().unwrap();
    /// assert_eq!(int.kind(), MySyntax::into_raw(Int));
    /// assert_eq!(int.as_token().unwrap().text(&interner), Some("42"));
    /// ```
    #[inline]
    pub fn with_interner(interner: &'i I) -> Self {
        Self {
            nodes: DashMap::default(),
            tokens: DashMap::default(),
            interner: MaybeOwnedRef::Borrowed(interner),
        }
    }

    /// Constructs a new, empty cache that will use the given interner to deduplicate source text
    /// (strings) across tokens.
    /// # Examples
    /// ```
    /// # use cstree::testing::*;
    /// # use cstree::interning::*;
    /// use cstree::build::NodeCache;
    ///
    /// // Create the builder from a custom interner
    /// let mut interner = new_interner();
    /// let cache = NodeCache::from_interner(interner);
    /// let mut builder: GreenNodeBuilder<MySyntax, TokenInterner> =
    ///     GreenNodeBuilder::from_cache(cache);
    ///
    /// // Construct the tree
    /// # builder.start_node(Root);
    /// # builder.token(Int, "42");
    /// # builder.finish_node();
    /// parse(&mut builder, "42");
    /// let (tree, cache) = builder.finish();
    ///
    /// // Use the tree
    /// let interner = cache.unwrap().into_interner().unwrap();
    /// assert_eq!(tree.kind(), MySyntax::into_raw(Root));
    /// let int = tree.children().next().unwrap();
    /// assert_eq!(int.kind(), MySyntax::into_raw(Int));
    /// assert_eq!(int.as_token().unwrap().text(&interner), Some("42"));
    /// ```
    #[inline]
    pub fn from_interner(interner: I) -> Self {
        Self {
            nodes: DashMap::default(),
            tokens: DashMap::default(),
            interner: MaybeOwnedRef::Owned(interner),
        }
    }

    /// Get a reference to the interner used to deduplicate source text (strings).
    ///
    /// See also [`interner_mut`](ThreadedNodeCache::interner_mut).
    #[inline]
    pub fn interner(&self) -> &I {
        &self.interner
    }

    /// Get a mutable reference to the interner used to deduplicate source text (strings).
    /// # Examples
    /// ```
    /// # use cstree::*;
    /// # use cstree::build::*;
    /// # use cstree::interning::*;
    /// let mut cache = NodeCache::new();
    /// let interner = cache.interner_mut();
    /// let key = interner.get_or_intern("foo");
    /// assert_eq!(interner.resolve(key), "foo");
    /// ```
    #[inline]
    pub fn interner_mut(&mut self) -> Option<&mut I> {
        match &mut self.interner {
            MaybeOwnedRef::Owned(interner) => Some(interner),
            MaybeOwnedRef::Borrowed(_) => None,
        }
    }

    /// If this node cache was constructed with [`new`](ThreadedNodeCache::new) or
    /// [`from_interner`](ThreadedNodeCache::from_interner), returns the interner used to deduplicate source
    /// text (strings) to allow resolving tree tokens back to text and re-using the interner to build
    /// additonal trees.
    #[inline]
    pub fn into_interner(self) -> Option<I> {
        self.interner.into_owned()
    }

    fn node<S: Syntax>(&self, kind: S, all_children: &mut Vec<GreenElement>, offset: usize) -> GreenNode {
        // NOTE: this fn must remove all children starting at `first_child` from `all_children` before returning
        let kind = S::into_raw(kind);
        let mut hasher = FxHasher32::default();
        let mut text_len: TextSize = 0.into();
        for child in &all_children[offset..] {
            text_len += child.text_len();
            child.hash(&mut hasher);
        }
        let child_hash = hasher.finish() as u32;

        // Green nodes are fully immutable, so it's ok to deduplicate them.
        // This is the same optimization that Roslyn does
        // https://github.com/KirillOsenkov/Bliki/wiki/Roslyn-Immutable-Trees
        //
        // For example, all `#[inline]` in this file share the same green node!
        // For `libsyntax/parse/parser.rs`, measurements show that deduping saves
        // 17% of the memory for green nodes!
        let children = all_children.drain(offset..);
        if children.len() <= CHILDREN_CACHE_THRESHOLD {
            self.get_cached_node(kind, children, text_len, child_hash)
        } else {
            GreenNode::new_with_len_and_hash(kind, children, text_len, child_hash)
        }
    }

    #[inline(always)]
    fn intern(&self, text: &str) -> TokenKey {
        (&*self.interner).get_or_intern(text)
    }

    /// Creates a [`GreenNode`] by looking inside the cache or inserting
    /// a new node into the cache if it's a cache miss.
    #[inline]
    fn get_cached_node(
        &self,
        kind: RawSyntaxKind,
        children: std::vec::Drain<'_, GreenElement>,
        text_len: TextSize,
        child_hash: u32,
    ) -> GreenNode {
        let head = GreenNodeHead {
            kind,
            text_len,
            child_hash,
        };
        self.nodes
            .entry(head.clone())
            .or_insert_with(|| GreenNode::from_head_and_children(head, children))
            .clone()
    }

    fn token<S: Syntax>(&self, kind: S, text: Option<TokenKey>, len: u32) -> GreenToken {
        let text_len = TextSize::from(len);
        let kind = S::into_raw(kind);
        let data = GreenTokenData { kind, text, text_len };
        self.tokens
            .entry(data.clone())
            .or_insert_with(|| GreenToken::new(data))
            .clone()
    }
}

pub use super::builder::Checkpoint;

/// A builder for green trees.
/// Construct with [`new`](ThreadedGreenNodeBuilder::new), [`with_cache`](ThreadedGreenNodeBuilder::with_cache), or
/// [`from_cache`](ThreadedGreenNodeBuilder::from_cache). To add tree nodes, start them with
/// [`start_node`](ThreadedGreenNodeBuilder::start_node), add [`token`](ThreadedGreenNodeBuilder::token)s and then
/// [`finish_node`](ThreadedGreenNodeBuilder::finish_node). When the whole tree is constructed, call
/// [`finish`](ThreadedGreenNodeBuilder::finish) to obtain the root.
///
/// # Examples
/// ```
/// # use cstree::testing::*;
/// // Build a tree
/// let mut builder: GreenNodeBuilder<MySyntax> = GreenNodeBuilder::new();
/// builder.start_node(Root);
/// builder.token(Int, "42");
/// builder.finish_node();
/// let (tree, cache) = builder.finish();
///
/// // Check it out!
/// assert_eq!(tree.kind(), MySyntax::into_raw(Root));
/// let int = tree.children().next().unwrap();
/// assert_eq!(int.kind(), MySyntax::into_raw(Int));
/// let resolver = cache.unwrap().into_interner().unwrap();
/// assert_eq!(int.as_token().unwrap().text(&resolver), Some("42"));
/// ```
#[derive(Debug)]
pub struct ThreadedGreenNodeBuilder<'cache, 'interner, S: Syntax, I = MultiThreadedTokenInterner> {
    cache: MaybeOwnedRef<'cache, ThreadedNodeCache<'interner, I>>,
    parents: Vec<(S, usize)>,
    children: Vec<GreenElement>,
    /// Caches the current document length to avoid recomputing it.
    doc_len: TextSize,
}

impl<S: Syntax> ThreadedGreenNodeBuilder<'static, 'static, S> {
    /// Creates new builder with an empty [`ThreadedNodeCache`].
    pub fn new() -> Self {
        Self {
            cache: MaybeOwnedRef::Owned(ThreadedNodeCache::new()),
            parents: Vec::with_capacity(8),
            children: Vec::with_capacity(8),
            doc_len: TextSize::new(0),
        }
    }
}

impl<S: Syntax> Default for ThreadedGreenNodeBuilder<'static, 'static, S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'cache, 'interner, S, I> ThreadedGreenNodeBuilder<'cache, 'interner, S, I>
where
    S: Syntax,
    for<'a> &'a I: Interner<TokenKey>,
{
    /// Reusing a [`ThreadedNodeCache`] between multiple builders saves memory, as it allows to structurally
    /// share underlying trees.
    pub fn with_cache(cache: &'cache ThreadedNodeCache<'interner, I>) -> Self {
        Self {
            cache: MaybeOwnedRef::Borrowed(cache),
            parents: Vec::with_capacity(8),
            children: Vec::with_capacity(8),
            doc_len: TextSize::new(0),
        }
    }

    /// Reusing a [`ThreadedNodeCache`] between multiple builders saves memory, as it allows to structurally
    /// share underlying trees.
    /// The `cache` given will be returned on [`finish`](ThreadedGreenNodeBuilder::finish).
    /// # Examples
    /// ```
    /// # use cstree::testing::*;
    /// # use cstree::build::*;
    /// // Construct a builder from our own cache
    /// let cache = NodeCache::new();
    /// let mut builder: GreenNodeBuilder<MySyntax> = GreenNodeBuilder::from_cache(cache);
    ///
    /// // Build a tree
    /// # builder.start_node(Root);
    /// # builder.token(Int, "42");
    /// # builder.finish_node();
    /// parse(&mut builder, "42");
    /// let (tree, cache) = builder.finish();
    ///
    /// // Use the tree
    /// let interner = cache.unwrap().into_interner().unwrap();
    /// assert_eq!(tree.kind(), MySyntax::into_raw(Root));
    /// let int = tree.children().next().unwrap();
    /// assert_eq!(int.kind(), MySyntax::into_raw(Int));
    /// assert_eq!(int.as_token().unwrap().text(&interner), Some("42"));
    /// ```
    pub fn from_cache(cache: ThreadedNodeCache<'interner, I>) -> Self {
        Self {
            cache: MaybeOwnedRef::Owned(cache),
            parents: Vec::with_capacity(8),
            children: Vec::with_capacity(8),
            doc_len: TextSize::new(0),
        }
    }

    /// Shortcut to construct a builder that uses an existing interner.
    ///
    /// This is equivalent to using [`from_cache`](ThreadedGreenNodeBuilder::from_cache) with a node cache
    /// obtained from [`ThreadedNodeCache::with_interner`].
    #[inline]
    pub fn with_interner(interner: &'interner mut I) -> Self {
        let cache = ThreadedNodeCache::with_interner(interner);
        Self::from_cache(cache)
    }

    /// Shortcut to construct a builder that uses an existing interner.
    ///
    /// This is equivalent to using [`from_cache`](ThreadedGreenNodeBuilder::from_cache) with a node cache
    /// obtained from [`ThreadedNodeCache::from_interner`].
    #[inline]
    pub fn from_interner(interner: I) -> Self {
        let cache = ThreadedNodeCache::from_interner(interner);
        Self::from_cache(cache)
    }

    /// Get a reference to the interner used to deduplicate source text (strings).
    ///
    /// This is the same interner as used by the underlying [`ThreadedNodeCache`].
    /// See also [`interner_mut`](ThreadedGreenNodeBuilder::interner_mut).
    #[inline]
    pub fn interner(&self) -> &I {
        &self.cache.interner
    }

    /// Get a mutable reference to the interner used to deduplicate source text (strings).
    ///
    /// This is the same interner as used by the underlying [`ThreadedNodeCache`].
    /// # Examples
    /// ```
    /// # use cstree::testing::*;
    /// # use cstree::build::*;
    /// # use cstree::interning::*;
    /// let mut builder: GreenNodeBuilder<MySyntax> = GreenNodeBuilder::new();
    /// let interner = builder.interner_mut();
    /// let key = interner.get_or_intern("foo");
    /// assert_eq!(interner.resolve(key), "foo");
    /// ```
    #[inline]
    pub fn interner_mut(&mut self) -> Option<&mut I> {
        match &mut self.cache {
            MaybeOwnedRef::Owned(cache) => cache.interner_mut(),
            MaybeOwnedRef::Borrowed(_) => None,
        }
    }

    /// Add a new token with the given `text` to the current node.
    ///
    /// ## Panics
    /// In debug mode, if `kind` has static text, this function will verify that `text` matches that text.
    #[inline]
    pub fn token(&mut self, kind: S, text: &str) {
        let token = match S::static_text(kind) {
            Some(static_text) => {
                debug_assert_eq!(
                    static_text, text,
                    r#"Received `{kind:?}` token which should have text "{static_text}", but "{text}" was given."#
                );
                self.cache.token::<S>(kind, None, static_text.len() as u32)
            }
            None => {
                let len = text.len() as u32;
                let text = self.cache.intern(text);
                self.cache.token::<S>(kind, Some(text), len)
            }
        };
        let text_len = token.text_len();
        self.children.push(token.into());
        self.doc_len += text_len;
    }

    /// Add a new token to the current node without storing an explicit section of text.
    /// This is useful if the text can always be inferred from the token's `kind`, for example
    /// when using kinds for specific operators or punctuation.
    ///
    /// For tokens whose textual representation is not static, such as numbers or identifiers, use
    /// [`token`](ThreadedGreenNodeBuilder::token).
    ///
    /// ## Panics
    /// If `kind` does not have static text, i.e., `L::static_text(kind)` returns `None`.
    #[inline]
    pub fn static_token(&mut self, kind: S) {
        let static_text = S::static_text(kind).unwrap_or_else(|| panic!("Missing static text for '{kind:?}'"));
        let token = self.cache.token::<S>(kind, None, static_text.len() as u32);
        let text_len = token.text_len();
        self.children.push(token.into());
        self.doc_len += text_len;
    }

    /// Start new node of the given `kind` and make it current.
    #[inline]
    pub fn start_node(&mut self, kind: S) {
        let len = self.children.len();
        self.parents.push((kind, len));
    }

    /// Finish the current branch and restore the previous branch as current.
    #[inline]
    pub fn finish_node(&mut self) {
        let (kind, first_child) = self.parents.pop().unwrap();
        // NOTE: we rely on the node cache to remove all children starting at `first_child` from `self.children`
        let node = self.cache.node::<S>(kind, &mut self.children, first_child);
        self.children.push(node.into());
    }

    /// Prepare for maybe wrapping the next node with a surrounding node.
    ///
    /// The way wrapping works is that you first get a checkpoint, then you add nodes and tokens as
    /// normal, and then you *maybe* call [`start_node_at`](ThreadedGreenNodeBuilder::start_node_at).
    ///
    /// # Examples
    /// ```
    /// # use cstree::testing::*;
    /// # use cstree::build::GreenNodeBuilder;
    /// # struct Parser;
    /// # impl Parser {
    /// #     fn peek(&self) -> Option<TestSyntaxKind> { None }
    /// #     fn parse_expr(&mut self) {}
    /// # }
    /// # let mut builder: GreenNodeBuilder<MySyntax> = GreenNodeBuilder::new();
    /// # let mut parser = Parser;
    /// let checkpoint = builder.checkpoint();
    /// parser.parse_expr();
    /// if let Some(Plus) = parser.peek() {
    ///     // 1 + 2 = Add(1, 2)
    ///     builder.start_node_at(checkpoint, Operation);
    ///     parser.parse_expr();
    ///     builder.finish_node();
    /// }
    /// ```
    #[inline]
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint(self.children.len())
    }

    /// Wrap the previous branch marked by [`checkpoint`](ThreadedGreenNodeBuilder::checkpoint) in a new
    /// branch and make it current.
    #[inline]
    pub fn start_node_at(&mut self, checkpoint: Checkpoint, kind: S) {
        let Checkpoint(checkpoint) = checkpoint;
        assert!(
            checkpoint <= self.children.len(),
            "checkpoint no longer valid, was finish_node called early?"
        );

        if let Some(&(_, first_child)) = self.parents.last() {
            assert!(
                checkpoint >= first_child,
                "checkpoint no longer valid, was an unmatched start_node_at called?"
            );
        }

        self.parents.push((kind, checkpoint));
    }

    /// Complete building the tree.
    ///
    /// Make sure that calls to [`start_node`](ThreadedGreenNodeBuilder::start_node) /
    /// [`start_node_at`](ThreadedGreenNodeBuilder::start_node_at) and
    /// [`finish_node`](ThreadedGreenNodeBuilder::finish_node) are balanced, i.e. that every started node has
    /// been completed!
    ///
    /// If this builder was constructed with [`new`](ThreadedGreenNodeBuilder::new) or
    /// [`from_cache`](ThreadedGreenNodeBuilder::from_cache), this method returns the cache used to deduplicate tree nodes
    ///  as its second return value to allow re-using the cache or extracting the underlying string
    ///  [`Interner`]. See also [`ThreadedNodeCache::into_interner`].
    #[inline]
    pub fn finish(mut self) -> (GreenNode, Option<ThreadedNodeCache<'interner, I>>) {
        assert_eq!(self.children.len(), 1);
        let cache = self.cache.into_owned();
        match self.children.pop().unwrap() {
            NodeOrToken::Node(node) => (node, cache),
            NodeOrToken::Token(_) => panic!("called `finish` on a `GreenNodeBuilder` which only contained a token"),
        }
    }

    /// Returns the children of the current node.
    #[inline]
    pub fn current_children(&self) -> &[GreenElement] {
        let first_child = self.parents.last().map_or(0, |&(_, first_child)| first_child);
        &self.children[first_child..]
    }

    /// Removes the last child from the current node and returns it.
    #[inline]
    pub fn pop_last_child(&mut self) -> GreenElement {
        assert!(
            !self.current_children().is_empty(),
            "no children to pop for current node"
        );
        let elem = self.children.pop().unwrap();
        self.doc_len -= elem.text_len();
        elem
    }

    /// Returns the length of text in the root node.
    #[inline]
    pub fn document_len(&self) -> TextSize {
        self.doc_len
    }
}
