use boa_engine::{Context, JsResult, Source};

pub fn register_collections(context: &mut Context) -> JsResult<()> {
    let script = r#"
        (function() {
            const HTMLCollection = globalThis.HTMLCollection;
            const NodeList = globalThis.NodeList;
            if (!HTMLCollection || !NodeList) return;

            const collectionFetchers = new WeakMap();
            const nodeListFetchers = new WeakMap();
            const childrenCache = new WeakMap();
            const childNodesCache = new WeakMap();
            const tagCollectionCaches = new WeakMap();
            const classCollectionCaches = new WeakMap();

            function indexFromKey(key) {
                if (typeof key !== 'string' || !/^(0|[1-9][0-9]*)$/.test(key)) return null;
                const index = Number(key);
                return Number.isSafeInteger(index) && index < 0xffffffff ? index : null;
            }

            function createLiveHTMLCollection(fetchElements) {
                const target = Object.create(HTMLCollection.prototype);
                let cachedRev = -1;
                let cachedItems = null;

                function getElements() {
                    const rev = typeof document.__getRevision === 'function' ? document.__getRevision() : -1;
                    if (cachedItems === null || cachedRev !== rev) {
                        cachedItems = fetchElements();
                        cachedRev = rev;
                    }
                    return cachedItems;
                }

                const proxy = new Proxy(target, {
                    get(target, key, receiver) {
                        if (key === 'length') {
                            return getElements().length;
                        }
                        const index = indexFromKey(key);
                        if (index !== null) {
                            const items = getElements();
                            return index < items.length ? items[index] : undefined;
                        }
                        if (typeof key === 'string' && !(key in target)) {
                            const items = getElements();
                            for (let i = 0; i < items.length; i++) {
                                const el = items[i];
                                if (el && typeof el.getAttribute === 'function') {
                                    if (el.getAttribute('id') === key || el.getAttribute('name') === key) {
                                        return el;
                                    }
                                }
                            }
                        }
                        return Reflect.get(target, key, receiver);
                    },
                    has(target, key) {
                        if (key === 'length') return true;
                        const index = indexFromKey(key);
                        if (index !== null) {
                            return index < getElements().length;
                        }
                        if (typeof key === 'string') {
                            const items = getElements();
                            for (let i = 0; i < items.length; i++) {
                                const el = items[i];
                                if (el && typeof el.getAttribute === 'function') {
                                    if (el.getAttribute('id') === key || el.getAttribute('name') === key) {
                                        return true;
                                    }
                                }
                            }
                        }
                        return Reflect.has(target, key);
                    },
                    ownKeys(target) {
                        const items = getElements();
                        return [...items.map((_, i) => String(i)), ...Reflect.ownKeys(target)];
                    },
                    getOwnPropertyDescriptor(target, key) {
                        const index = indexFromKey(key);
                        if (index !== null) {
                            const items = getElements();
                            if (index < items.length) {
                                return { value: items[index], enumerable: true, configurable: true, writable: false };
                            }
                            return undefined;
                        }
                        return Reflect.getOwnPropertyDescriptor(target, key);
                    },
                    set(target, key, value, receiver) {
                        if (key === 'length' || indexFromKey(key) !== null) return false;
                        return Reflect.set(target, key, value, receiver);
                    },
                    deleteProperty(target, key) {
                        if (key === 'length' || indexFromKey(key) !== null) return false;
                        return Reflect.deleteProperty(target, key);
                    }
                });
                collectionFetchers.set(proxy, getElements);
                collectionFetchers.set(target, getElements);
                return proxy;
            }

            function createNodeList(fetchNodes, isLive) {
                const target = Object.create(NodeList.prototype);
                let cachedRev = -1;
                let cachedNodes = null;
                const getNodes = isLive ? function() {
                    const rev = typeof document.__getRevision === 'function' ? document.__getRevision() : -1;
                    if (cachedNodes === null || cachedRev !== rev) {
                        cachedNodes = fetchNodes();
                        cachedRev = rev;
                    }
                    return cachedNodes;
                } : (function(snapshot) { return function() { return snapshot; }; })(fetchNodes());

                const proxy = new Proxy(target, {
                    get(target, key, receiver) {
                        if (key === 'length') {
                            return getNodes().length;
                        }
                        const index = indexFromKey(key);
                        if (index !== null) {
                            const nodes = getNodes();
                            return index < nodes.length ? nodes[index] : undefined;
                        }
                        return Reflect.get(target, key, receiver);
                    },
                    has(target, key) {
                        if (key === 'length') return true;
                        const index = indexFromKey(key);
                        if (index !== null) {
                            return index < getNodes().length;
                        }
                        return Reflect.has(target, key);
                    },
                    ownKeys(target) {
                        const nodes = getNodes();
                        return [...nodes.map((_, i) => String(i)), ...Reflect.ownKeys(target)];
                    },
                    getOwnPropertyDescriptor(target, key) {
                        const index = indexFromKey(key);
                        if (index !== null) {
                            const nodes = getNodes();
                            if (index < nodes.length) {
                                return { value: nodes[index], enumerable: true, configurable: true, writable: false };
                            }
                            return undefined;
                        }
                        return Reflect.getOwnPropertyDescriptor(target, key);
                    },
                    set(target, key, value, receiver) {
                        if (key === 'length' || indexFromKey(key) !== null) return false;
                        return Reflect.set(target, key, value, receiver);
                    },
                    deleteProperty(target, key) {
                        if (key === 'length' || indexFromKey(key) !== null) return false;
                        return Reflect.deleteProperty(target, key);
                    }
                });
                nodeListFetchers.set(proxy, getNodes);
                nodeListFetchers.set(target, getNodes);
                return proxy;
            }

            HTMLCollection.prototype.item = function item(index) {
                const fetch = collectionFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                const number = Number(index);
                if (!Number.isFinite(number) || number < 0) return null;
                const pos = Math.trunc(number) >>> 0;
                const items = fetch();
                return pos < items.length ? items[pos] : null;
            };
            HTMLCollection.prototype.namedItem = function namedItem(name) {
                const fetch = collectionFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                if (name == null || name === '') return null;
                const strName = String(name);
                const items = fetch();
                for (let i = 0; i < items.length; i++) {
                    const el = items[i];
                    if (el && typeof el.getAttribute === 'function') {
                        if (el.getAttribute('id') === strName || el.getAttribute('name') === strName) {
                            return el;
                        }
                    }
                }
                return null;
            };
            Object.defineProperty(HTMLCollection.prototype, 'length', {
                get() {
                    const fetch = collectionFetchers.get(this);
                    if (!fetch) throw new TypeError('Illegal invocation');
                    return fetch().length;
                },
                enumerable: true,
                configurable: true,
            });
            HTMLCollection.prototype[Symbol.iterator] = function iterator() {
                const fetch = collectionFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                return fetch().values();
            };

            NodeList.prototype.item = function item(index) {
                const fetch = nodeListFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                const number = Number(index);
                if (!Number.isFinite(number) || number < 0) return null;
                const pos = Math.trunc(number) >>> 0;
                const nodes = fetch();
                return pos < nodes.length ? nodes[pos] : null;
            };
            Object.defineProperty(NodeList.prototype, 'length', {
                get() {
                    const fetch = nodeListFetchers.get(this);
                    if (!fetch) throw new TypeError('Illegal invocation');
                    return fetch().length;
                },
                enumerable: true,
                configurable: true,
            });
            NodeList.prototype.forEach = function forEach(callback, thisArg) {
                const fetch = nodeListFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                if (typeof callback !== 'function') throw new TypeError('NodeList.forEach requires a callback');
                const nodes = fetch();
                for (let i = 0; i < nodes.length; i++) {
                    callback.call(thisArg, nodes[i], i, this);
                }
            };
            NodeList.prototype.keys = function keys() {
                const fetch = nodeListFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                return fetch().keys();
            };
            NodeList.prototype.values = function values() {
                const fetch = nodeListFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                return fetch().values();
            };
            NodeList.prototype.entries = function entries() {
                const fetch = nodeListFetchers.get(this);
                if (!fetch) throw new TypeError('Illegal invocation');
                return fetch().entries();
            };
            NodeList.prototype[Symbol.iterator] = NodeList.prototype.values;

            Object.defineProperty(Node.prototype, 'childNodes', {
                get() {
                    if (!this || !(this instanceof Node)) throw new TypeError('Illegal invocation');
                    let list = childNodesCache.get(this);
                    if (!list) {
                        list = createNodeList(() => this.__getChildNodes(), true);
                        childNodesCache.set(this, list);
                    }
                    return list;
                },
                enumerable: true,
                configurable: true,
            });

            const parents = [Element.prototype, Document.prototype, DocumentFragment.prototype];
            for (const proto of parents) {
                Object.defineProperty(proto, 'children', {
                    get() {
                        if (!this || !(this instanceof Node)) throw new TypeError('Illegal invocation');
                        let coll = childrenCache.get(this);
                        if (!coll) {
                            coll = createLiveHTMLCollection(() => this.__getChildren());
                            childrenCache.set(this, coll);
                        }
                        return coll;
                    },
                    enumerable: true,
                    configurable: true,
                });
            }

            function getElementsByTagName(tag) {
                if (!this || !(this instanceof Node)) throw new TypeError('Illegal invocation');
                const tagName = tag === undefined ? '' : String(tag);
                let cache = tagCollectionCaches.get(this);
                if (!cache) {
                    cache = new Map();
                    tagCollectionCaches.set(this, cache);
                }
                const cacheKey = tagName.toLowerCase();
                let coll = cache.get(cacheKey);
                if (!coll) {
                    coll = createLiveHTMLCollection(() => this.__getElementsByTagName(tagName));
                    cache.set(cacheKey, coll);
                }
                return coll;
            }
            Element.prototype.getElementsByTagName = getElementsByTagName;
            Document.prototype.getElementsByTagName = getElementsByTagName;

            function getElementsByClassName(className) {
                if (!this || !(this instanceof Node)) throw new TypeError('Illegal invocation');
                const classStr = className === undefined ? '' : String(className);
                let cache = classCollectionCaches.get(this);
                if (!cache) {
                    cache = new Map();
                    classCollectionCaches.set(this, cache);
                }
                const cacheKey = classStr.split(/\s+/).filter(Boolean).sort().join(' ');
                let coll = cache.get(cacheKey);
                if (!coll) {
                    coll = createLiveHTMLCollection(() => this.__getElementsByClassName(classStr));
                    cache.set(cacheKey, coll);
                }
                return coll;
            }
            Element.prototype.getElementsByClassName = getElementsByClassName;
            Document.prototype.getElementsByClassName = getElementsByClassName;

            function querySelectorAll(selector) {
                if (!this || !(this instanceof Node)) throw new TypeError('Illegal invocation');
                const sel = selector === undefined ? '' : String(selector);
                return createNodeList(() => this.__querySelectorAll(sel), false);
            }
            Element.prototype.querySelectorAll = querySelectorAll;
            Document.prototype.querySelectorAll = querySelectorAll;
            DocumentFragment.prototype.querySelectorAll = querySelectorAll;

            const scriptsCache = new WeakMap();
            const imagesCache = new WeakMap();
            const formsCache = new WeakMap();
            const linksCache = new WeakMap();

            Object.defineProperty(Document.prototype, 'scripts', {
                get() {
                    if (!this || !(this instanceof Document)) throw new TypeError('Illegal invocation');
                    let coll = scriptsCache.get(this);
                    if (!coll) {
                        coll = this.getElementsByTagName('script');
                        scriptsCache.set(this, coll);
                    }
                    return coll;
                },
                enumerable: true,
                configurable: true,
            });
            Object.defineProperty(Document.prototype, 'images', {
                get() {
                    if (!this || !(this instanceof Document)) throw new TypeError('Illegal invocation');
                    let coll = imagesCache.get(this);
                    if (!coll) {
                        coll = this.getElementsByTagName('img');
                        imagesCache.set(this, coll);
                    }
                    return coll;
                },
                enumerable: true,
                configurable: true,
            });
            Object.defineProperty(Document.prototype, 'forms', {
                get() {
                    if (!this || !(this instanceof Document)) throw new TypeError('Illegal invocation');
                    let coll = formsCache.get(this);
                    if (!coll) {
                        coll = this.getElementsByTagName('form');
                        formsCache.set(this, coll);
                    }
                    return coll;
                },
                enumerable: true,
                configurable: true,
            });
            Object.defineProperty(Document.prototype, 'links', {
                get() {
                    if (!this || !(this instanceof Document)) throw new TypeError('Illegal invocation');
                    let coll = linksCache.get(this);
                    if (!coll) {
                        coll = createLiveHTMLCollection(() => {
                            const allElements = this.__getElementsByTagName('*');
                            const links = [];
                            for (let i = 0; i < allElements.length; i++) {
                                const el = allElements[i];
                                const tag = (el.tagName || '').toUpperCase();
                                if ((tag === 'A' || tag === 'AREA') && typeof el.hasAttribute === 'function' && el.hasAttribute('href')) {
                                    links.push(el);
                                }
                            }
                            return links;
                        });
                        linksCache.set(this, coll);
                    }
                    return coll;
                },
                enumerable: true,
                configurable: true,
            });
        })();
    "#;
    context.eval(Source::from_bytes(script.as_bytes()))?;
    Ok(())
}
