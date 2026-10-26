use super::*;

pub(super) fn register_dom_traversal(context: &mut Context) -> JsResult<()> {
    crate::structured_clone::register_platform_objects(context, r#"
        (function(markNonCloneable) {
            const NodeFilter = () => { throw new TypeError('Illegal constructor: NodeFilter'); };
            Object.defineProperty(NodeFilter, 'name', {value: 'NodeFilter'});
            const constants = {
                FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3,
                SHOW_ALL: 0xFFFFFFFF, SHOW_ELEMENT: 0x1, SHOW_ATTRIBUTE: 0x2,
                SHOW_TEXT: 0x4, SHOW_CDATA_SECTION: 0x8, SHOW_ENTITY_REFERENCE: 0x10,
                SHOW_ENTITY: 0x20, SHOW_PROCESSING_INSTRUCTION: 0x40, SHOW_COMMENT: 0x80,
                SHOW_DOCUMENT: 0x100, SHOW_DOCUMENT_TYPE: 0x200, SHOW_DOCUMENT_FRAGMENT: 0x400,
            };
            for (const [name, value] of Object.entries(constants)) {
                Object.defineProperty(NodeFilter, name, {value, enumerable: true});
            }
            globalThis.NodeFilter = NodeFilter;

            const nativeParent = Node.prototype.__getParentNode;
            const nativeFirst = Node.prototype.__getFirstChild;
            const nativeLast = Node.prototype.__getLastChild;
            const nativeNext = Node.prototype.__getNextSibling;
            const nativePrevious = Node.prototype.__getPrevSibling;
            const parent = node => nativeParent.call(node);
            const first = node => nativeFirst.call(node);
            const last = node => nativeLast.call(node);
            const next = node => nativeNext.call(node);
            const previous = node => nativePrevious.call(node);
            const walkers = new WeakMap();
            const Exception = DOMException;
            function state(walker) {
                const state = walkers.get(walker);
                if (!state) throw new TypeError('Illegal TreeWalker receiver');
                return state;
            }
            function filterNode(state, node) {
                if (state.active) {
                    throw new Exception('A TreeWalker filter cannot recursively filter nodes', 'InvalidStateError');
                }
                if (!(state.whatToShow & (1 << (node.nodeType - 1)))) return 3;
                if (state.filter === null) return 1;
                state.active = true;
                try {
                    const callback = state.filter;
                    const value = typeof callback === 'function'
                        ? callback.call(undefined, node)
                        : callback.acceptNode.call(callback, node);
                    return (+value) & 0xFFFF;
                } finally {
                    state.active = false;
                }
            }
            function accept(state, node) {
                state.current = node;
                return node;
            }
            function traverseChildren(walker, childOf, siblingOf) {
                const s = state(walker);
                let node = childOf(s.current);
                while (node !== null) {
                    const result = filterNode(s, node);
                    if (result === 1) return accept(s, node);
                    if (result === 3) {
                        const child = childOf(node);
                        if (child !== null) { node = child; continue; }
                    }
                    while (node !== null) {
                        const sibling = siblingOf(node);
                        if (sibling !== null) { node = sibling; break; }
                        const ancestor = parent(node);
                        if (ancestor === null || ancestor === s.root || ancestor === s.current) return null;
                        node = ancestor;
                    }
                }
                return null;
            }
            function traverseSiblings(walker, childOf, siblingOf) {
                const s = state(walker);
                let node = s.current;
                if (node === s.root) return null;
                while (true) {
                    let sibling = siblingOf(node);
                    while (sibling !== null) {
                        node = sibling;
                        const result = filterNode(s, node);
                        if (result === 1) return accept(s, node);
                        sibling = childOf(node);
                        if (result === 2 || sibling === null) sibling = siblingOf(node);
                    }
                    node = parent(node);
                    if (node === null || node === s.root) return null;
                    if (filterNode(s, node) === 1) return null;
                }
            }
            class TreeWalker {
                constructor() { throw new TypeError('Illegal constructor: TreeWalker'); }
                get root() { return state(this).root; }
                get whatToShow() { return state(this).whatToShow; }
                get filter() { return state(this).filter; }
                get currentNode() { return state(this).current; }
                set currentNode(node) {
                    const s = state(this);
                    if (!(node instanceof Node)) throw new TypeError('currentNode must be a Node');
                    parent(node);
                    s.current = node;
                }
                parentNode() {
                    const s = state(this);
                    let node = s.current;
                    while (node !== null && node !== s.root) {
                        node = parent(node);
                        if (node !== null && filterNode(s, node) === 1) return accept(s, node);
                    }
                    return null;
                }
                firstChild() { return traverseChildren(this, first, next); }
                lastChild() { return traverseChildren(this, last, previous); }
                nextSibling() { return traverseSiblings(this, first, next); }
                previousSibling() { return traverseSiblings(this, last, previous); }
                nextNode() {
                    const s = state(this);
                    let node = s.current;
                    let result = 1;
                    while (true) {
                        let child;
                        while (result !== 2 && (child = first(node)) !== null) {
                            node = child;
                            result = filterNode(s, node);
                            if (result === 1) return accept(s, node);
                        }
                        let sibling = null;
                        let temporary = node;
                        while (temporary !== null) {
                            if (temporary === s.root) return null;
                            sibling = next(temporary);
                            if (sibling !== null) break;
                            temporary = parent(temporary);
                        }
                        if (sibling === null) return null;
                        node = sibling;
                        result = filterNode(s, node);
                        if (result === 1) return accept(s, node);
                    }
                }
                previousNode() {
                    const s = state(this);
                    let node = s.current;
                    while (node !== s.root) {
                        let sibling = previous(node);
                        while (sibling !== null) {
                            node = sibling;
                            let result = filterNode(s, node);
                            let child;
                            while (result !== 2 && (child = last(node)) !== null) {
                                node = child;
                                result = filterNode(s, node);
                            }
                            if (result === 1) return accept(s, node);
                            sibling = previous(node);
                        }
                        const ancestor = parent(node);
                        if (node === s.root || ancestor === null) return null;
                        node = ancestor;
                        if (filterNode(s, node) === 1) return accept(s, node);
                    }
                    return null;
                }
            }
            for (const name of Object.getOwnPropertyNames(TreeWalker.prototype)) {
                if (name === 'constructor') continue;
                const descriptor = Object.getOwnPropertyDescriptor(TreeWalker.prototype, name);
                descriptor.enumerable = true;
                Object.defineProperty(TreeWalker.prototype, name, descriptor);
            }
            Object.defineProperty(TreeWalker.prototype, Symbol.toStringTag, {value: 'TreeWalker', configurable: true});
            globalThis.TreeWalker = TreeWalker;
            Object.defineProperty(Document.prototype, 'createTreeWalker', {
                configurable: true, writable: true, enumerable: true,
                value: function createTreeWalker(root, whatToShow = 0xFFFFFFFF, filter = null) {
                    if (!(this instanceof Document)) throw new TypeError('Illegal Document receiver');
                    if (!(root instanceof Node)) throw new TypeError('TreeWalker root must be a Node');
                    parent(this);
                    parent(root);
                    const mask = (+whatToShow) >>> 0;
                    if (filter !== null && typeof filter !== 'object' && typeof filter !== 'function') {
                        throw new TypeError('TreeWalker filter must be an object');
                    }
                    const walker = Object.create(TreeWalker.prototype);
                    markNonCloneable(walker);
                    walkers.set(walker, {root, current: root, whatToShow: mask, filter, active: false});
                    return walker;
                },
            });
        })
    "#)?;
    Ok(())
}
