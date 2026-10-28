(function () {
  "use strict";


  var STORAGE_KEY = "rix_browser_state_v4";

  function defaultDials() {
    return [];
  }

  function defaultBookmarks() {
    return [];
  }

  function defaultHistory() {
    return [];
  }

  function defaultDownloads() {
    return [];
  }

  function initialData() {
    var t1 = { id: 1, url: "rix://newtab", title: "New Tab", pinned: false };
    return {
      activeTabId: 1,
      nextTabId: 2,
      tabs: [t1],
      closedTabs: [],
      bookmarks: defaultBookmarks(),
      history: defaultHistory(),
      downloads: defaultDownloads(),
      dials: defaultDials(),
      settings: {
        theme: "dark",
        sidebar: "open",
        density: "regular",
        searchEngine: "google",
      },
    };
  }

  function loadState() {
    try {
      var raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        var parsed = JSON.parse(raw);
        if (parsed && Array.isArray(parsed.tabs) && parsed.tabs.length > 0) {
          return parsed;
        }
      }
    } catch (e) {
      console.warn("Failed to parse state from localStorage:", e);
    }
    return initialData();
  }

  var state = loadState();

  function saveState() {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch (e) {
      console.warn("Failed to persist state:", e);
    }
  }


  var $ = function (id) { return document.getElementById(id); };

  var shell = $("browser-shell");
  var sidebar = $("rix-sidebar");
  var tabListPinned = $("tab-list-pinned");
  var tabListOpen = $("tab-list-open");
  var pinnedHeader = $("pinned-header");
  var openHeader = $("open-header");
  var omnibox = $("omnibox");
  var omniboxSuggestions = $("omnibox-suggestions");
  var suggestionsList = $("suggestions-list");
  var internalViewport = $("internal-viewport");
  var webSurface = $("active-web-surface");
  var loadProgress = $("load-progress");
  var btnStar = $("btn-star");
  var moreMenu = $("more-menu");
  var commandPalette = $("command-palette");
  var paletteInput = $("palette-input");
  var paletteResults = $("palette-results");
  var flashEl = $("rix-flash");
  var flashText = $("rix-flash-text");
  var contextMenu = $("tab-context-menu");

  var activeContextMenuTabId = null;

  function flash(msg) {
    if (!flashEl || !flashText) return;
    flashText.textContent = msg;
    flashEl.removeAttribute("hidden");
    clearTimeout(flashEl._timer);
    flashEl._timer = setTimeout(function () {
      flashEl.setAttribute("hidden", "true");
    }, 2400);
  }

  function getDomain(url) {
    if (!url) return "";
    try {
      var u = new URL(url);
      return u.hostname.replace(/^www\./, "");
    } catch (e) {
      if (url.indexOf("://") !== -1) {
        var part = url.split("://")[1].split("/")[0].split("?")[0];
        return part.replace(/^www\./, "");
      }
      return url;
    }
  }

  function escapeHtml(str) {
    if (!str) return "";
    return String(str)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }

  function getMonogram(title, url) {
    if (title && title.trim().length > 0) {
      var first = title.trim()[0];
      if (/^[a-zA-Z0-9]/i.test(first)) {
        return first.toUpperCase();
      }
    }
    if (url) {
      try {
        var u = new URL(url);
        var host = u.hostname.replace(/^www\./, "");
        if (host.length > 0) return host[0].toUpperCase();
      } catch (e) {
        if (url.indexOf("://") !== -1) {
          var part = url.split("://")[1];
          if (part && part.length > 0) return part[0].toUpperCase();
        }
      }
    }
    return "R";
  }

  function renderTabIcon(tab) {
    var url = tab.url || "";
    if (url.indexOf("rix://") === 0 || url === "about:blank") {
      var iconWrap = document.createElement("span");
      iconWrap.className = "tab-item-icon-svg";
      if (url === "rix://newtab" || url === "about:blank") {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z"/></svg>';
      } else if (url === "rix://bookmarks") {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M17 3H7c-1.1 0-1.99.9-1.99 2L5 21l7-3 7 3V5c0-1.1-.9-2-2-2z"/></svg>';
      } else if (url === "rix://history") {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"/></svg>';
      } else if (url === "rix://downloads") {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96zM17 13l-5 5-5-5h3V9h4v4h3z"/></svg>';
      } else if (url === "rix://shortcuts") {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M20 5H4c-1.1 0-1.99.9-1.99 2L2 17c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm-9 3h2v2h-2V8zm0 3h2v2h-2v-2zM8 8h2v2H8V8zm0 3h2v2H8v-2zm-1 2H5v-2h2v2zm0-3H5V8h2v2zm8 7H9v-2h6v2zm1-4h-2v-2h2v2zm0-3h-2V8h2v2zm3 3h-2v-2h2v2zm0-3h-2V8h2v2z"/></svg>';
      } else if (url === "rix://settings") {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/></svg>';
      } else {
        iconWrap.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z"/></svg>';
      }
      return iconWrap;
    }

    var hostname = getDomain(url).toLowerCase();
    if (hostname.indexOf("youtube.com") !== -1 || hostname.indexOf("youtu.be") !== -1) {
      var yt = document.createElement("span");
      yt.className = "tab-item-icon-svg";
      yt.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24" style="color:#ff0000"><path fill="currentColor" d="m10 15 5.19-3L10 9v6m11.56-7.83c.13.47.22 1.1.28 1.9.07.8.1 1.49.1 2.09L22 12c0 2.19-.16 3.8-.44 4.83-.25.9-.83 1.48-1.73 1.73-.47.13-1.33.22-2.65.28-1.3.07-2.49.1-3.59.1L12 19c-4.19 0-6.8-.16-7.83-.44-.9-.25-1.48-.83-1.73-1.73-.13-.47-.22-1.1-.28-1.9-.07-.8-.1-1.49-.1-2.09L2 12c0-2.19.16-3.8.44-4.83.25-.9.83-1.48 1.73-1.73.47-.13 1.33-.22 2.65-.28 1.3-.07 2.49-.1 3.59-.1L12 5c4.19 0 6.8.16 7.83.44.9.25 1.48.83 1.73 1.73Z"/></svg>';
      return yt;
    }
    if (hostname.indexOf("github.com") !== -1) {
      var gh = document.createElement("span");
      gh.className = "tab-item-icon-svg";
      gh.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M12 2A10 10 0 0 0 2 12c0 4.42 2.87 8.17 6.84 9.5.5.08.66-.23.66-.5v-1.69c-2.77.6-3.36-1.34-3.36-1.34-.46-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.87 1.52 2.34 1.07 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.92 0-1.11.38-2 1.03-2.71-.1-.25-.45-1.29.1-2.64 0 0 .84-.27 2.75 1.02.79-.22 1.65-.33 2.5-.33.85 0 1.71.11 2.5.33 1.91-1.29 2.75-1.02 2.75-1.02.55 1.35.2 2.39.1 2.64.65.71 1.03 1.6 1.03 2.71 0 3.82-2.34 4.66-4.57 4.91.36.31.69.92.69 1.85V21c0 .27.16.59.67.5C19.14 20.16 22 16.42 22 12A10 10 0 0 0 12 2Z"/></svg>';
      return gh;
    }
    if (hostname.indexOf("google.com") !== -1) {
      var gg = document.createElement("span");
      gg.className = "tab-item-icon-svg";
      gg.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24" style="color:#4285f4"><path fill="currentColor" d="M21.35 11.1h-9.17v2.73h5.51c-.33 1.62-1.76 2.77-3.56 2.77-2.18 0-3.95-1.78-3.95-3.97s1.77-3.97 3.95-3.97c1.03 0 1.95.38 2.65 1.01l2.06-2.06C17.15 6.47 14.82 5.5 12.18 5.5 7.66 5.5 4 9.16 4 13.68s3.66 8.18 8.18 8.18c4.72 0 7.85-3.32 7.85-8 0-.58-.06-1.14-.18-1.76z"/></svg>';
      return gg;
    }
    if (hostname.indexOf("reddit.com") !== -1) {
      var rd = document.createElement("span");
      rd.className = "tab-item-icon-svg";
      rd.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24" style="color:#ff4500"><path fill="currentColor" d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5.72 13.25c-.21.73-1.08 1.48-2.6 1.89-1.02.27-2.22.42-3.12.42s-2.1-.15-3.12-.42c-1.52-.41-2.39-1.16-2.6-1.89-.13-.44.13-.91.57-1.04.44-.13.91.13 1.04.57.1.36.66.9 1.88 1.23.82.22 1.84.34 2.23.34.39 0 1.41-.12 2.23-.34 1.22-.33 1.78-.87 1.88-1.23.13-.44.6-.7 1.04-.57.44.13.7.6.57 1.04z"/></svg>';
      return rd;
    }
    if (hostname.indexOf("wikipedia.org") !== -1) {
      var wk = document.createElement("span");
      wk.className = "tab-item-icon-svg";
      wk.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M12.09 2.91 8.55 13.6 5.24 2.91H2.47l4.7 14.88h2.64l3.52-10.74 3.52 10.74h2.64l4.7-14.88h-2.77l-3.31 10.69-3.54-10.69z"/></svg>';
      return wk;
    }

    var container = document.createElement("span");
    container.className = "tab-item-icon-container";

    var img = document.createElement("img");
    img.className = "tab-item-favicon";
    img.src = "https://www.google.com/s2/favicons?domain=" + encodeURIComponent(hostname) + "&sz=32";
    img.alt = "";

    var fallback = document.createElement("span");
    fallback.className = "tab-item-monogram";
    fallback.style.display = "none";
    fallback.textContent = getMonogram(tab.title, tab.url);

    img.onerror = function () {
      img.style.display = "none";
      fallback.style.display = "flex";
    };

    container.appendChild(img);
    container.appendChild(fallback);
    return container;
  }

  function isInternalUrl(url) {
    if (!url) return false;
    return url.indexOf("rix://") === 0 || url === "about:blank";
  }

  function getSearchEngineUrl(query) {
    var engine = (state.settings && state.settings.searchEngine) || "google";
    var encoded = encodeURIComponent(query);
    if (engine === "duckduckgo") {
      return "https://duckduckgo.com/?q=" + encoded;
    } else if (engine === "bing") {
      return "https://www.bing.com/search?q=" + encoded;
    }
    return "https://www.google.com/search?q=" + encoded;
  }

  function normalizeInput(input) {
    var raw = input ? input.trim() : "";
    if (!raw) return "rix://newtab";
    if (raw.indexOf("rix://") === 0 || raw.indexOf("http://") === 0 || raw.indexOf("https://") === 0) {
      return raw;
    }
    if (/^[a-zA-Z0-9-]+\.[a-zA-Z]{2,}(\/.*)?$/.test(raw)) {
      return "https://" + raw;
    }
    return getSearchEngineUrl(raw);
  }


  function getActiveTab() {
    var tab = state.tabs.find(function (t) { return t.id === state.activeTabId; });
    return tab || state.tabs[0] || null;
  }

  function renderTabs() {
    if (!tabListPinned || !tabListOpen) return;

    tabListPinned.innerHTML = "";
    tabListOpen.innerHTML = "";

    var pinnedCount = 0;
    var openCount = 0;

    state.tabs.forEach(function (tab) {
      var item = document.createElement("div");
      item.className = "rix-tab-item" + (tab.id === state.activeTabId ? " active" : "");
      item.setAttribute("data-tab-id", String(tab.id));
      item.setAttribute("title", tab.title || "New Tab");

      var iconEl = renderTabIcon(tab);

      var title = document.createElement("span");
      title.className = "tab-item-title";
      title.textContent = tab.title || "New Tab";

      var closeBtn = document.createElement("button");
      closeBtn.className = "tab-item-close";
      closeBtn.setAttribute("title", "Close tab (Ctrl+W)");
      closeBtn.innerHTML = '<svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="m6.4 5 5.6 5.6L17.6 5 19 6.4 13.4 12l5.6 5.6-1.4 1.4-5.6-5.6L6.4 19 5 17.6l5.6-5.6L5 6.4 6.4 5Z"/></svg>';

      closeBtn.addEventListener("click", function (ev) {
        ev.stopPropagation();
        closeTab(tab.id);
      });

      item.appendChild(iconEl);
      item.appendChild(title);
      item.appendChild(closeBtn);

      item.addEventListener("click", function () {
        switchTab(tab.id);
      });

      item.addEventListener("contextmenu", function (ev) {
        ev.preventDefault();
        showContextMenu(tab.id, ev.clientX, ev.clientY);
      });

      if (tab.pinned) {
        tabListPinned.appendChild(item);
        pinnedCount++;
      } else {
        tabListOpen.appendChild(item);
        openCount++;
      }
    });

    if (pinnedHeader) {
      pinnedHeader.style.display = pinnedCount > 0 ? "flex" : "none";
    }

    updateToolbarBookmarkState();
  }

  function switchTab(id) {
    if (state.activeTabId === id) return;
    state.activeTabId = id;

    var tab = getActiveTab();
    if (tab && omnibox) {
      omnibox.value = tab.url;
    }

    var host = window.rix || window.rixChrome;
    if (host && host.tabs && typeof host.tabs.switch === "function") {
      try { host.tabs.switch(id); } catch (e) {}
    }

    saveState();
    renderTabs();
    updateViewForActiveTab();
  }

  function createTab(url, focusOmni) {
    var targetUrl = url ? url : "rix://newtab";
    var newId = state.nextTabId++;
    var title = "New Tab";
    if (targetUrl === "rix://bookmarks") title = "Bookmarks";
    else if (targetUrl === "rix://history") title = "History";
    else if (targetUrl === "rix://downloads") title = "Downloads";
    else if (targetUrl === "rix://shortcuts") title = "Shortcuts";
    else if (targetUrl === "rix://settings") title = "Settings";
    else if (targetUrl.indexOf("http") === 0) title = targetUrl;

    var newTabObj = {
      id: newId,
      url: targetUrl,
      title: title,
      pinned: false,
    };

    state.tabs.push(newTabObj);
    state.activeTabId = newId;

    var host = window.rix || window.rixChrome;
    if (host && host.tabs && typeof host.tabs.create === "function") {
      try { host.tabs.create({ url: targetUrl }); } catch (e) {}
    }

    saveState();
    renderTabs();
    updateViewForActiveTab();

    if (focusOmni && omnibox) {
      omnibox.focus();
      omnibox.select();
    }
  }

  function duplicateTab(id) {
    var tab = state.tabs.find(function (t) { return t.id === id; });
    if (!tab) tab = getActiveTab();
    if (tab) {
      createTab(tab.url, false);
      flash("Tab duplicated");
    }
  }

  function togglePinTab(id) {
    var tab = state.tabs.find(function (t) { return t.id === id; });
    if (!tab) tab = getActiveTab();
    if (tab) {
      tab.pinned = !tab.pinned;
      saveState();
      renderTabs();
      flash(tab.pinned ? "Tab pinned" : "Tab unpinned");
    }
  }

  function closeTab(id) {
    var index = state.tabs.findIndex(function (t) { return t.id === id; });
    if (index === -1) return;

    var closed = state.tabs.splice(index, 1)[0];
    state.closedTabs.push(closed);
    if (state.closedTabs.length > 20) state.closedTabs.shift();

    if (state.tabs.length === 0) {
      createTab("rix://newtab", false);
      return;
    }

    if (state.activeTabId === id) {
      var nextIndex = index >= state.tabs.length ? state.tabs.length - 1 : index;
      state.activeTabId = state.tabs[nextIndex].id;
    }

    var host = window.rix || window.rixChrome;
    if (host && host.tabs && typeof host.tabs.close === "function") {
      try { host.tabs.close(id); } catch (e) {}
    }

    saveState();
    renderTabs();
    updateViewForActiveTab();
  }

  function closeOtherTabs(keepId) {
    var host = window.rix || window.rixChrome;
    var toClose = state.tabs.filter(function (t) {
      return t.id !== keepId && !t.pinned;
    });
    toClose.forEach(function (t) {
      state.closedTabs.push(t);
      if (host && host.tabs && typeof host.tabs.close === "function") {
        try { host.tabs.close(t.id); } catch (e) {}
      }
    });
    if (state.closedTabs.length > 30) {
      state.closedTabs = state.closedTabs.slice(-30);
    }
    state.tabs = state.tabs.filter(function (t) {
      return t.id === keepId || t.pinned;
    });
    state.activeTabId = keepId;
    saveState();
    renderTabs();
    updateViewForActiveTab();
    flash("Other tabs closed");
  }

  function closeTabsToRight(fromId) {
    var targetIdx = state.tabs.findIndex(function (t) { return t.id === fromId; });
    if (targetIdx === -1) return;
    var host = window.rix || window.rixChrome;
    var remaining = [];
    var closed = [];
    for (var i = 0; i < state.tabs.length; i++) {
      var t = state.tabs[i];
      if (i <= targetIdx || t.pinned) {
        remaining.push(t);
      } else {
        closed.push(t);
        if (host && host.tabs && typeof host.tabs.close === "function") {
          try { host.tabs.close(t.id); } catch (e) {}
        }
      }
    }
    if (closed.length === 0) return;
    state.closedTabs = state.closedTabs.concat(closed);
    if (state.closedTabs.length > 30) state.closedTabs = state.closedTabs.slice(-30);
    state.tabs = remaining;
    if (!state.tabs.some(function (t) { return t.id === state.activeTabId; })) {
      state.activeTabId = fromId;
    }
    saveState();
    renderTabs();
    updateViewForActiveTab();
    flash("Closed " + closed.length + " tabs to the right");
  }

  function reopenClosedTab() {
    if (state.closedTabs.length === 0) {
      flash("No closed tabs to reopen");
      return;
    }
    var tab = state.closedTabs.pop();
    createTab(tab.url, false);
  }


  function navigateTo(url) {
    var targetUrl = normalizeInput(url);
    var tab = getActiveTab();
    if (!tab) return;

    tab.url = targetUrl;
    if (targetUrl === "rix://newtab") tab.title = "New Tab";
    else if (targetUrl === "rix://bookmarks") tab.title = "Bookmarks";
    else if (targetUrl === "rix://history") tab.title = "History";
    else if (targetUrl === "rix://downloads") tab.title = "Downloads";
    else if (targetUrl === "rix://shortcuts") tab.title = "Shortcuts";
    else if (targetUrl === "rix://settings") tab.title = "Settings";
    else tab.title = targetUrl;

    if (omnibox) omnibox.value = targetUrl;

    var host = window.rix || window.rixChrome;
    if (host && host.navigation && typeof host.navigation.goTo === "function") {
      try { host.navigation.goTo(targetUrl); } catch (e) {}
    }

    recordHistory(targetUrl, tab.title);

    saveState();
    renderTabs();
    updateViewForActiveTab();
    closeSuggestions();
  }

  function updateViewForActiveTab() {
    var tab = getActiveTab();
    if (!tab) return;

    if (omnibox) {
      omnibox.value = tab.url;
    }

    var secBadge = $("security-badge");
    if (secBadge) {
      if (isInternalUrl(tab.url)) {
        secBadge.textContent = "Internal";
        secBadge.style.display = "inline-block";
      } else if (tab.url.indexOf("https://") === 0) {
        secBadge.textContent = "Secure";
        secBadge.style.display = "inline-block";
      } else {
        secBadge.style.display = "none";
      }
    }

    if (isInternalUrl(tab.url)) {
      if (internalViewport) {
        internalViewport.style.display = "block";
        renderInternalPage(tab.url);
      }
    } else {
      if (internalViewport) {
        internalViewport.style.display = "none";
        internalViewport.innerHTML = "";
        internalViewport._renderedUrl = null;
      }
    }

    updateToolbarBookmarkState();
    updateFooterNavActiveState(tab.url);
  }

  function updateFooterNavActiveState(url) {
    var navIds = [
      { id: "nav-btn-bookmarks", url: "rix://bookmarks" },
      { id: "nav-btn-history", url: "rix://history" },
      { id: "nav-btn-downloads", url: "rix://downloads" },
      { id: "nav-btn-shortcuts", url: "rix://shortcuts" },
      { id: "nav-btn-settings", url: "rix://settings" },
    ];
    navIds.forEach(function (item) {
      var btn = $(item.id);
      if (btn) {
        if (url === item.url) btn.classList.add("active");
        else btn.classList.remove("active");
      }
    });
  }


  function showContextMenu(tabId, x, y) {
    if (!contextMenu) return;
    activeContextMenuTabId = tabId;

    var tab = state.tabs.find(function (t) { return t.id === tabId; });
    var pinLabel = $("ctx-pin-label");
    if (pinLabel && tab) {
      pinLabel.textContent = tab.pinned ? "Unpin Tab" : "Pin Tab";
    }

    var menuW = 200;
    var menuH = 260;
    var rawX = typeof x === "number" && !isNaN(x) && x > 0 ? x : 60;
    var rawY = typeof y === "number" && !isNaN(y) && y > 0 ? y : 130;
    var isCollapsed = state.settings.sidebar === "collapsed" || (sidebar && sidebar.classList.contains("collapsed"));
    if (isCollapsed) {
      rawX = Math.max(rawX, 52);
    }
    var posX = Math.min(rawX, (window.innerWidth || 1280) - menuW - 10);
    var posY = Math.min(rawY, (window.innerHeight || 900) - menuH - 10);

    contextMenu.setAttribute("style", "left: " + Math.max(10, Math.round(posX)) + "px; top: " + Math.max(10, Math.round(posY)) + "px;");
    contextMenu.removeAttribute("hidden");
  }

  function hideContextMenu() {
    if (contextMenu) {
      contextMenu.setAttribute("hidden", "true");
    }
    activeContextMenuTabId = null;
  }

  function setupContextMenuEvents() {
    var ctxReload = $("ctx-reload-tab");
    if (ctxReload) {
      ctxReload.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) {
          var tab = state.tabs.find(function (t) { return t.id === activeContextMenuTabId; });
          if (tab) {
            if (activeContextMenuTabId === state.activeTabId) {
              var host = window.rix || window.rixChrome;
              if (host && host.navigation && typeof host.navigation.reload === "function") {
                try { host.navigation.reload(); } catch (e) {}
              }
            }
            navigateTo(tab.url);
            flash("Tab reloaded");
          }
        }
        hideContextMenu();
      });
    }

    var ctxPin = $("ctx-pin-tab");
    if (ctxPin) {
      ctxPin.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) togglePinTab(activeContextMenuTabId);
        hideContextMenu();
      });
    }

    var ctxDuplicate = $("ctx-duplicate-tab");
    if (ctxDuplicate) {
      ctxDuplicate.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) duplicateTab(activeContextMenuTabId);
        hideContextMenu();
      });
    }

    var ctxCopyUrl = $("ctx-copy-url");
    if (ctxCopyUrl) {
      ctxCopyUrl.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) {
          var tab = state.tabs.find(function (t) { return t.id === activeContextMenuTabId; });
          if (tab) {
            var urlToCopy = tab.url;
            var host = window.rix || window.rixChrome;
            if (host && host.clipboard && typeof host.clipboard.writeText === "function") {
              try { host.clipboard.writeText(urlToCopy); } catch (e) {}
            } else if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
              try { navigator.clipboard.writeText(urlToCopy); } catch (e) {}
            }
            flash("URL copied to clipboard");
          }
        }
        hideContextMenu();
      });
    }

    var ctxCloseTab = $("ctx-close-tab");
    if (ctxCloseTab) {
      ctxCloseTab.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) closeTab(activeContextMenuTabId);
        hideContextMenu();
      });
    }

    var ctxCloseOthers = $("ctx-close-others");
    if (ctxCloseOthers) {
      ctxCloseOthers.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) closeOtherTabs(activeContextMenuTabId);
        hideContextMenu();
      });
    }

    var ctxCloseRight = $("ctx-close-right");
    if (ctxCloseRight) {
      ctxCloseRight.addEventListener("click", function () {
        if (activeContextMenuTabId !== null) closeTabsToRight(activeContextMenuTabId);
        hideContextMenu();
      });
    }

    var ctxReopen = $("ctx-reopen-tab");
    if (ctxReopen) {
      ctxReopen.addEventListener("click", function () {
        reopenClosedTab();
        hideContextMenu();
      });
    }

    window.addEventListener("click", function (e) {
      if (contextMenu && !contextMenu.contains(e.target)) {
        hideContextMenu();
      }
    });

    window.addEventListener("scroll", hideContextMenu, true);
  }


  function recordHistory(url, title) {
    if (!url || isInternalUrl(url)) return;
    state.history = state.history.filter(function (h) { return h.url !== url; });
    state.history.unshift({
      id: "h_" + Date.now(),
      title: title || url,
      url: url,
      visitedAt: Date.now(),
    });
    if (state.history.length > 200) state.history.pop();
  }

  function toggleBookmarkCurrentTab() {
    var tab = getActiveTab();
    if (!tab) return;
    var existingIndex = state.bookmarks.findIndex(function (b) { return b.url === tab.url; });
    if (existingIndex !== -1) {
      state.bookmarks.splice(existingIndex, 1);
      flash("Bookmark removed");
    } else {
      state.bookmarks.unshift({
        id: "b_" + Date.now(),
        title: tab.title || tab.url,
        url: tab.url,
        folder: "Favorites",
        createdAt: Date.now(),
      });
      flash("Bookmark added");
    }
    saveState();
    updateToolbarBookmarkState();
  }

  function updateToolbarBookmarkState() {
    var tab = getActiveTab();
    if (!btnStar) return;
    var isBookmarked = tab && state.bookmarks.some(function (b) { return b.url === tab.url; });
    var starIcon = btnStar.querySelector(".icon-star");
    if (starIcon) {
      if (isBookmarked) starIcon.classList.add("active");
      else starIcon.classList.remove("active");
    }
  }


  function renderInternalPage(url, force) {
    if (!internalViewport) return;

    var normalized = url ? url : "rix://newtab";
    if (normalized === "about:blank") normalized = "rix://newtab";

    if (!force && internalViewport._renderedUrl === normalized && internalViewport.children.length > 0) {
      return;
    }
    internalViewport._renderedUrl = normalized;
    internalViewport.innerHTML = "";

    if (normalized === "rix://newtab") {
      renderNewTab(internalViewport);
    } else if (normalized === "rix://bookmarks") {
      renderBookmarksPage(internalViewport);
    } else if (normalized === "rix://history") {
      renderHistoryPage(internalViewport);
    } else if (normalized === "rix://downloads") {
      renderDownloadsPage(internalViewport);
    } else if (normalized === "rix://shortcuts") {
      renderShortcutsPage(internalViewport);
    } else if (normalized === "rix://settings") {
      renderSettingsPage(internalViewport);
    } else {
      renderNotFoundPage(internalViewport, normalized);
    }
  }

  function renderNewTab(container) {
    var wrap = document.createElement("div");
    wrap.className = "newtab-container";

    var now = new Date();
    var hours = now.getHours();
    var greeting = "Welcome";
    if (hours >= 5 && hours < 12) greeting = "Good morning";
    else if (hours >= 12 && hours < 18) greeting = "Good afternoon";
    else if (hours >= 18 && hours < 23) greeting = "Good evening";
    else greeting = "Good night";

    var dateStr = now.toLocaleDateString("en-US", {
      weekday: "long",
      year: "numeric",
      month: "long",
      day: "numeric",
    });

    var h = String(now.getHours()).padStart(2, "0");
    var m = String(now.getMinutes()).padStart(2, "0");
    var s = String(now.getSeconds()).padStart(2, "0");

    var clockSec = document.createElement("div");
    clockSec.className = "newtab-clock-section";
    clockSec.innerHTML =
      '<div class="newtab-clock" id="live-clock">' + h + ":" + m + '<span style="font-size:32px;opacity:0.6">:' + s + '</span></div>' +
      '<div class="newtab-date">' + dateStr + '</div>' +
      '<div class="newtab-greeting">' + greeting + '</div>';

    wrap.appendChild(clockSec);

    clearInterval(window._clockInterval);
    window._clockInterval = setInterval(function () {
      var liveClock = $("live-clock");
      if (!liveClock) return;
      var cur = new Date();
      var hh = String(cur.getHours()).padStart(2, "0");
      var mm = String(cur.getMinutes()).padStart(2, "0");
      var ss = String(cur.getSeconds()).padStart(2, "0");
      liveClock.innerHTML = hh + ":" + mm + '<span style="font-size:32px;opacity:0.6">:' + ss + '</span>';
    }, 1000);

    var form = document.createElement("form");
    form.className = "newtab-search-form";
    form.innerHTML =
      '<div class="newtab-search-box">' +
        '<svg class="ui-icon newtab-search-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg>' +
        '<input type="text" class="newtab-search-input" id="newtab-input" placeholder="Search Google or enter address" autocomplete="off" />' +
      '</div>';

    form.addEventListener("submit", function (e) {
      e.preventDefault();
      var input = $("newtab-input");
      if (input && input.value.trim().length > 0) {
        navigateTo(input.value.trim());
      }
    });

    var newtabInput = form.querySelector("#newtab-input");
    if (newtabInput) {
      newtabInput.addEventListener("keydown", function (e) {
        if (e.key === "Enter") {
          e.preventDefault();
          if (newtabInput.value.trim().length > 0) {
            navigateTo(newtabInput.value.trim());
          }
        }
      });
    }

    wrap.appendChild(form);

    var dialsSec = document.createElement("div");
    dialsSec.className = "speed-dials-section";
    dialsSec.id = "speed-dials-section";

    var headRow = document.createElement("div");
    headRow.className = "section-head-row";
    headRow.innerHTML =
      '<span class="section-label">Quick Access</span>' +
      '<div style="display:flex;gap:12px;">' +
        '<span class="section-btn-link" id="add-dial-btn">+ Add Shortcut</span>' +
        '<span class="section-btn-link" id="edit-dials-btn">Edit</span>' +
      '</div>';

    dialsSec.appendChild(headRow);

    var grid = document.createElement("div");
    grid.className = "speed-dials-grid";
    grid.id = "speed-dials-grid";

    renderDialsIntoGrid(grid);
    dialsSec.appendChild(grid);
    wrap.appendChild(dialsSec);

    if (state.history.length > 0) {
      var histSec = document.createElement("div");
      histSec.className = "recent-history-section";
      histSec.innerHTML =
        '<div class="section-head-row">' +
          '<span class="section-label">Recent History</span>' +
          '<span class="section-btn-link" id="open-all-history">View all</span>' +
        '</div>';

      var histList = document.createElement("div");
      histList.className = "recent-history-list";

      var recentItems = state.history.slice(0, 4);
      recentItems.forEach(function (h) {
        var card = document.createElement("div");
        card.className = "history-card-item";
        card.innerHTML =
          '<div class="history-card-monogram">' + getMonogram(h.title, h.url) + '</div>' +
          '<div class="history-card-info">' +
            '<div class="history-card-title">' + (h.title || h.url) + '</div>' +
            '<div class="history-card-url">' + h.url + '</div>' +
          '</div>';

        card.addEventListener("click", function () {
          navigateTo(h.url);
        });
        histList.appendChild(card);
      });

      histSec.appendChild(histList);
      wrap.appendChild(histSec);
    }

    container.appendChild(wrap);

    var addDialBtn = $("add-dial-btn");
    if (addDialBtn) {
      addDialBtn.addEventListener("click", function () {
        var url = prompt("Enter URL:", "https://");
        if (!url || !url.trim()) return;
        var title = prompt("Enter Title:", "Shortcut");
        state.dials.push({
          id: "d_" + Date.now(),
          title: title || url,
          url: url.trim(),
        });
        saveState();
        var g = $("speed-dials-grid");
        if (g) renderDialsIntoGrid(g);
        flash("Shortcut added");
      });
    }

    var editDialsBtn = $("edit-dials-btn");
    if (editDialsBtn) {
      editDialsBtn.addEventListener("click", function () {
        var sec = $("speed-dials-section");
        if (sec) {
          sec.classList.toggle("editing");
          editDialsBtn.textContent = sec.classList.contains("editing") ? "Done" : "Edit";
        }
      });
    }

    var openAllHist = $("open-all-history");
    if (openAllHist) {
      openAllHist.addEventListener("click", function () {
        navigateTo("rix://history");
      });
    }
  }

  function renderDialsIntoGrid(grid) {
    grid.innerHTML = "";
    state.dials.forEach(function (dial) {
      var card = document.createElement("div");
      card.className = "dial-card";
      card.innerHTML =
        '<div class="dial-monogram">' + getMonogram(dial.title, dial.url) + '</div>' +
        '<div class="dial-title">' + dial.title + '</div>' +
        '<button class="dial-remove-btn" title="Remove">&times;</button>';

      card.addEventListener("click", function (e) {
        if (e.target.classList.contains("dial-remove-btn")) {
          e.stopPropagation();
          state.dials = state.dials.filter(function (d) { return d.id !== dial.id; });
          saveState();
          renderDialsIntoGrid(grid);
          flash("Shortcut removed");
          return;
        }
        navigateTo(dial.url);
      });

      grid.appendChild(card);
    });
  }

  function renderBookmarksPage(container) {
    var wrap = document.createElement("div");
    wrap.className = "bookmarks-container";

    var headerRow = document.createElement("div");
    headerRow.className = "page-header-row";
    headerRow.innerHTML =
      '<h1 class="page-header-title">Bookmarks</h1>' +
      '<button class="primary-btn" id="btn-add-bookmark">+ Add Bookmark</button>';
    wrap.appendChild(headerRow);

    var activeFolder = "All";
    var folders = ["All", "Favorites", "Reading", "Work", "Later"];
    var chipsRow = document.createElement("div");
    chipsRow.className = "filter-chips-row";

    folders.forEach(function (f) {
      var chip = document.createElement("button");
      chip.className = "chip-btn" + (f === activeFolder ? " active" : "");
      chip.textContent = f;
      chip.addEventListener("click", function () {
        activeFolder = f;
        chipsRow.querySelectorAll(".chip-btn").forEach(function (b) { b.classList.remove("active"); });
        chip.classList.add("active");
        renderBookmarksList();
      });
      chipsRow.appendChild(chip);
    });
    wrap.appendChild(chipsRow);

    var listWrap = document.createElement("div");
    listWrap.className = "bookmarks-list";
    wrap.appendChild(listWrap);

    function renderBookmarksList() {
      listWrap.innerHTML = "";
      var filtered = activeFolder === "All"
        ? state.bookmarks
        : state.bookmarks.filter(function (b) { return b.folder === activeFolder; });

      if (filtered.length === 0) {
        listWrap.innerHTML = '<div style="padding:48px 24px;text-align:center;color:var(--muted);font-size:14px;"><svg class="ui-icon" style="width:36px;height:36px;margin:0 auto 12px;opacity:0.4;display:block;" viewBox="0 0 24 24"><path fill="currentColor" d="M17 3H7c-1.1 0-1.99.9-1.99 2L5 21l7-3 7 3V5c0-1.1-.9-2-2-2z"/></svg>No bookmarks saved yet.<br><span style="font-size:12px;opacity:0.7">Click the star icon or press Ctrl+D to bookmark any page.</span></div>';
        return;
      }

      filtered.forEach(function (b) {
        var item = document.createElement("div");
        item.className = "bookmark-item";
        item.innerHTML =
          '<span class="tab-item-monogram">' + getMonogram(b.title, b.url) + '</span>' +
          '<div class="bookmark-item-info">' +
            '<div class="bookmark-item-title">' + b.title + '</div>' +
            '<div class="bookmark-item-url">' + b.url + '</div>' +
          '</div>' +
          '<span class="bookmark-item-tag">' + (b.folder || "Favorites") + '</span>' +
          '<button class="delete-action-btn" title="Delete bookmark">&times;</button>';

        item.addEventListener("click", function (e) {
          if (e.target.classList.contains("delete-action-btn")) {
            e.stopPropagation();
            state.bookmarks = state.bookmarks.filter(function (x) { return x.id !== b.id; });
            saveState();
            renderBookmarksList();
            updateToolbarBookmarkState();
            flash("Bookmark deleted");
            return;
          }
          navigateTo(b.url);
        });

        listWrap.appendChild(item);
      });
    }

    renderBookmarksList();
    container.appendChild(wrap);

    var addBtn = $("btn-add-bookmark");
    if (addBtn) {
      addBtn.addEventListener("click", function () {
        var url = prompt("Enter bookmark URL:", "https://");
        if (!url || !url.trim()) return;
        var title = prompt("Enter title:", url);
        state.bookmarks.unshift({
          id: "b_" + Date.now(),
          title: title || url,
          url: url.trim(),
          folder: activeFolder === "All" ? "Favorites" : activeFolder,
          createdAt: Date.now(),
        });
        saveState();
        renderBookmarksList();
        updateToolbarBookmarkState();
        flash("Bookmark added");
      });
    }
  }

  function renderHistoryPage(container) {
    var wrap = document.createElement("div");
    wrap.className = "history-container";

    var headerRow = document.createElement("div");
    headerRow.className = "page-header-row";
    headerRow.innerHTML =
      '<h1 class="page-header-title">History</h1>' +
      '<button class="primary-btn" id="btn-clear-history" style="background-color:var(--danger)">Clear History</button>';
    wrap.appendChild(headerRow);

    var searchInput = document.createElement("input");
    searchInput.type = "text";
    searchInput.id = "history-search-input";
    searchInput.className = "history-search-input";
    searchInput.placeholder = "Search history...";
    wrap.appendChild(searchInput);

    var listWrap = document.createElement("div");
    listWrap.className = "bookmarks-list";
    wrap.appendChild(listWrap);

    function renderList(query) {
      listWrap.innerHTML = "";
      var q = (query || "").toLowerCase().trim();
      var filtered = state.history.filter(function (h) {
        return !q || h.title.toLowerCase().indexOf(q) !== -1 || h.url.toLowerCase().indexOf(q) !== -1;
      });

      if (filtered.length === 0) {
        listWrap.innerHTML = '<div style="padding:48px 24px;text-align:center;color:var(--muted);font-size:14px;"><svg class="ui-icon" style="width:36px;height:36px;margin:0 auto 12px;opacity:0.4;display:block;" viewBox="0 0 24 24"><path fill="currentColor" d="M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"/></svg>No history entries found</div>';
        return;
      }

      filtered.forEach(function (h) {
        var item = document.createElement("div");
        item.className = "bookmark-item";
        var date = new Date(h.visitedAt);
        var timeStr = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

        item.innerHTML =
          '<span class="tab-item-monogram">' + getMonogram(h.title, h.url) + '</span>' +
          '<div class="bookmark-item-info">' +
            '<div class="bookmark-item-title">' + (h.title || h.url) + '</div>' +
            '<div class="bookmark-item-url">' + h.url + '</div>' +
          '</div>' +
          '<span class="bookmark-item-tag">' + timeStr + '</span>' +
          '<button class="delete-action-btn" title="Remove from history">&times;</button>';

        item.addEventListener("click", function (e) {
          if (e.target.classList.contains("delete-action-btn")) {
            e.stopPropagation();
            state.history = state.history.filter(function (x) { return x.id !== h.id; });
            saveState();
            renderList(searchInput.value);
            flash("Item removed from history");
            return;
          }
          navigateTo(h.url);
        });

        listWrap.appendChild(item);
      });
    }

    renderList("");
    container.appendChild(wrap);

    searchInput.addEventListener("input", function () {
      renderList(searchInput.value);
    });

    searchInput.addEventListener("keydown", function (e) {
      if (e.key === "Enter") {
        renderList(searchInput.value);
      }
    });

    var clearBtn = $("btn-clear-history");
    if (clearBtn) {
      clearBtn.addEventListener("click", function () {
        state.history = [];
        saveState();
        var host = window.rix || window.rixChrome;
        if (host && host.history && typeof host.history.clear === "function") {
          try { host.history.clear(); } catch (e) {}
        }
        renderList("");
        flash("History cleared");
      });
    }
  }

  function renderDownloadsPage(container) {
    var wrap = document.createElement("div");
    wrap.className = "downloads-container";

    var headerRow = document.createElement("div");
    headerRow.className = "page-header-row";
    headerRow.innerHTML =
      '<h1 class="page-header-title">Downloads</h1>' +
      '<button class="primary-btn" id="btn-clear-downloads">Clear List</button>';
    wrap.appendChild(headerRow);

    var listWrap = document.createElement("div");
    listWrap.className = "downloads-list";

    function renderList() {
      listWrap.innerHTML = "";
      if (state.downloads.length === 0) {
        listWrap.innerHTML = '<div style="padding:48px 24px;text-align:center;color:var(--muted);font-size:14px;"><svg class="ui-icon" style="width:36px;height:36px;margin:0 auto 12px;opacity:0.4;display:block;" viewBox="0 0 24 24"><path fill="currentColor" d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96zM17 13l-5 5-5-5h3V9h4v4h3z"/></svg>No downloads yet.</div>';
        return;
      }
      state.downloads.forEach(function (d) {
        var item = document.createElement("div");
        item.className = "download-item";
        item.innerHTML =
          '<svg class="ui-icon" style="width:24px;height:24px;color:var(--accent)" viewBox="0 0 24 24"><path fill="currentColor" d="M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96zM17 13l-5 5-5-5h3V9h4v4h3z"/></svg>' +
          '<div class="download-item-info">' +
            '<div class="download-item-title">' + d.name + '</div>' +
            '<div class="download-item-meta">' + d.size + ' • ' + d.date + '</div>' +
          '</div>' +
          '<span class="download-status-badge">' + d.status + '</span>';
        listWrap.appendChild(item);
      });
    }

    renderList();
    wrap.appendChild(listWrap);
    container.appendChild(wrap);

    var clearBtn = $("btn-clear-downloads");
    if (clearBtn) {
      clearBtn.addEventListener("click", function () {
        state.downloads = [];
        saveState();
        renderList();
        flash("Downloads cleared");
      });
    }
  }

  function renderShortcutsPage(container) {
    var wrap = document.createElement("div");
    wrap.className = "shortcuts-container";

    wrap.innerHTML =
      '<div class="page-header-row">' +
        '<h1 class="page-header-title">Keyboard Shortcuts</h1>' +
      '</div>' +
      '<div class="shortcuts-grid">' +
        '<div class="shortcut-group-card">' +
          '<div class="shortcut-group-title">Navigation</div>' +
          '<div class="shortcut-list">' +
            '<div class="shortcut-item"><span>Focus Address Bar</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>L</kbd></div></div>' +
            '<div class="shortcut-item"><span>Command Palette</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>K</kbd></div></div>' +
            '<div class="shortcut-item"><span>Back</span><div class="shortcut-keys"><kbd>Alt</kbd><kbd>&larr;</kbd></div></div>' +
            '<div class="shortcut-item"><span>Forward</span><div class="shortcut-keys"><kbd>Alt</kbd><kbd>&rarr;</kbd></div></div>' +
            '<div class="shortcut-item"><span>Reload</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>R</kbd></div></div>' +
          '</div>' +
        '</div>' +
        '<div class="shortcut-group-card">' +
          '<div class="shortcut-group-title">Tabs</div>' +
          '<div class="shortcut-list">' +
            '<div class="shortcut-item"><span>New Tab</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>T</kbd></div></div>' +
            '<div class="shortcut-item"><span>Close Tab</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>W</kbd></div></div>' +
            '<div class="shortcut-item"><span>Reopen Closed Tab</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>T</kbd></div></div>' +
            '<div class="shortcut-item"><span>Next Tab</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>Tab</kbd></div></div>' +
          '</div>' +
        '</div>' +
        '<div class="shortcut-group-card">' +
          '<div class="shortcut-group-title">Views</div>' +
          '<div class="shortcut-list">' +
            '<div class="shortcut-item"><span>Bookmarks</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>D</kbd></div></div>' +
            '<div class="shortcut-item"><span>History</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>H</kbd></div></div>' +
            '<div class="shortcut-item"><span>Downloads</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>J</kbd></div></div>' +
            '<div class="shortcut-item"><span>Toggle Sidebar</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>B</kbd></div></div>' +
          '</div>' +
        '</div>' +
        '<div class="shortcut-group-card">' +
          '<div class="shortcut-group-title">General</div>' +
          '<div class="shortcut-list">' +
            '<div class="shortcut-item"><span>Settings</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>,</kbd></div></div>' +
            '<div class="shortcut-item"><span>Shortcuts Help</span><div class="shortcut-keys"><kbd>Ctrl</kbd><kbd>/</kbd></div></div>' +
            '<div class="shortcut-item"><span>Close Modal</span><div class="shortcut-keys"><kbd>Esc</kbd></div></div>' +
          '</div>' +
        '</div>' +
      '</div>';

    container.appendChild(wrap);
  }

  function renderSettingsPage(container) {
    var wrap = document.createElement("div");
    wrap.className = "settings-container";

    wrap.innerHTML =
      '<div class="page-header-row">' +
        '<h1 class="page-header-title">Settings</h1>' +
      '</div>' +
      '<div class="settings-card">' +
        '<div class="settings-card-title">Appearance</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Theme</span>' +
            '<span class="setting-desc">Select visual color theme</span>' +
          '</div>' +
          '<div class="segmented-control" id="theme-control">' +
            '<button class="segment-btn' + (state.settings.theme === "dark" ? " active" : "") + '" data-val="dark">Dark</button>' +
            '<button class="segment-btn' + (state.settings.theme === "light" ? " active" : "") + '" data-val="light">Light</button>' +
          '</div>' +
        '</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Sidebar Mode</span>' +
            '<span class="setting-desc">Choose sidebar expanded or compact rail</span>' +
          '</div>' +
          '<div class="segmented-control" id="sidebar-control">' +
            '<button class="segment-btn' + (state.settings.sidebar === "open" ? " active" : "") + '" data-val="open">Expanded</button>' +
            '<button class="segment-btn' + (state.settings.sidebar === "rail" ? " active" : "") + '" data-val="rail">Compact Rail</button>' +
          '</div>' +
        '</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Density</span>' +
            '<span class="setting-desc">Control spacing and compactness</span>' +
          '</div>' +
          '<div class="segmented-control" id="density-control">' +
            '<button class="segment-btn' + (state.settings.density === "regular" ? " active" : "") + '" data-val="regular">Spacious</button>' +
            '<button class="segment-btn' + (state.settings.density === "compact" ? " active" : "") + '" data-val="compact">Compact</button>' +
          '</div>' +
        '</div>' +
      '</div>' +
      '<div class="settings-card">' +
        '<div class="settings-card-title">Search Engine</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Default Provider</span>' +
            '<span class="setting-desc">Engine used for address bar queries</span>' +
          '</div>' +
          '<div class="segmented-control" id="search-control">' +
            '<button class="segment-btn' + (state.settings.searchEngine === "google" ? " active" : "") + '" data-val="google">Google</button>' +
            '<button class="segment-btn' + (state.settings.searchEngine === "duckduckgo" ? " active" : "") + '" data-val="duckduckgo">DuckDuckGo</button>' +
            '<button class="segment-btn' + (state.settings.searchEngine === "bing" ? " active" : "") + '" data-val="bing">Bing</button>' +
          '</div>' +
        '</div>' +
      '</div>' +
      '<div class="settings-card">' +
        '<div class="settings-card-title">Privacy & Cookies</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Stored Website Data</span>' +
            '<span class="setting-desc">Clear cookies and storage for specific domains</span>' +
          '</div>' +
          '<button class="primary-btn" id="btn-clear-all-cookies" style="background-color:var(--danger)">Clear All Cookies</button>' +
        '</div>' +
        '<div class="cookie-domains-list" id="settings-cookies-list" style="margin-top:14px;display:flex;flex-direction:column;gap:8px;"></div>' +
      '</div>' +
      '<div class="settings-card">' +
        '<div class="settings-card-title">Backup & Reset</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Export Data</span>' +
            '<span class="setting-desc">Download bookmarks, settings and shortcuts</span>' +
          '</div>' +
          '<button class="primary-btn" id="btn-export-json">Export JSON</button>' +
        '</div>' +
        '<div class="settings-row">' +
          '<div class="settings-row-label">' +
            '<span class="setting-name">Reset Browser</span>' +
            '<span class="setting-desc">Restore all settings and default dials</span>' +
          '</div>' +
          '<button class="primary-btn" id="btn-reset-browser" style="background-color:var(--danger)">Reset to Defaults</button>' +
        '</div>' +
      '</div>';

    container.appendChild(wrap);

    setupSegmentControl("theme-control", function (val) {
      state.settings.theme = val;
      applyTheme(val);
      saveState();
      flash("Theme set to " + (val === "dark" ? "Dark" : "Light"));
    });

    setupSegmentControl("sidebar-control", function (val) {
      state.settings.sidebar = val;
      applySidebarMode(val);
      saveState();
    });

    setupSegmentControl("density-control", function (val) {
      state.settings.density = val;
      if (shell) shell.setAttribute("data-density", val);
      saveState();
      flash("Density set to " + val);
    });

    setupSegmentControl("search-control", function (val) {
      state.settings.searchEngine = val;
      saveState();
      flash("Search engine: " + val);
    });

    var clearAllCookiesBtn = $("btn-clear-all-cookies");
    function renderSettingsCookiesList() {
      var listEl = $("settings-cookies-list");
      if (!listEl) return;
      listEl.innerHTML = "";
      var host = window.rix || window.rixChrome;
      var domains = [];
      if (host && host.cookies && typeof host.cookies.list === "function") {
        try { domains = host.cookies.list(); } catch (e) {}
      }
      if (domains.length === 0) {
        listEl.innerHTML = '<div style="font-size:13px;color:var(--muted);padding:8px 0;">No website cookies stored</div>';
        return;
      }
      domains.forEach(function (item) {
        var row = document.createElement("div");
        row.style.cssText = "display:flex;justify-content:space-between;align-items:center;padding:8px 12px;background:var(--elevated);border:1px solid var(--line);border-radius:8px;";
        row.innerHTML =
          '<div>' +
            '<div style="font-size:13px;font-weight:500;color:var(--fg);">' + escapeHtml(item.domain) + '</div>' +
            '<div style="font-size:11px;color:var(--muted);">' + item.count + ' cookies</div>' +
          '</div>' +
          '<button class="delete-action-btn" data-domain="' + item.domain + '" title="Delete cookies for ' + item.domain + '">&times;</button>';
        row.querySelector(".delete-action-btn").addEventListener("click", function () {
          if (host && host.cookies && typeof host.cookies.deleteForDomain === "function") {
            try { host.cookies.deleteForDomain(item.domain); } catch (e) {}
          }
          flash("Cookies deleted for " + item.domain);
          renderSettingsCookiesList();
        });
        listEl.appendChild(row);
      });
    }

    if (clearAllCookiesBtn) {
      clearAllCookiesBtn.addEventListener("click", function () {
        var host = window.rix || window.rixChrome;
        if (host && host.cookies && typeof host.cookies.clearAll === "function") {
          try { host.cookies.clearAll(); } catch (e) {}
        }
        renderSettingsCookiesList();
        flash("All cookies cleared");
      });
    }
    renderSettingsCookiesList();

    var exportBtn = $("btn-export-json");
    if (exportBtn) {
      exportBtn.addEventListener("click", function () {
        var blob = new Blob([JSON.stringify(state, null, 2)], { type: "application/json" });
        var a = document.createElement("a");
        a.href = URL.createObjectURL(blob);
        a.download = "rix-browser-backup.json";
        a.click();
        flash("Backup exported");
      });
    }

    var resetBtn = $("btn-reset-browser");
    if (resetBtn) {
      resetBtn.addEventListener("click", function () {
        if (confirm("Reset all settings and restore defaults?")) {
          localStorage.removeItem(STORAGE_KEY);
          state = initialData();
          saveState();
          applyTheme(state.settings.theme);
          applySidebarMode(state.settings.sidebar);
          renderTabs();
          updateViewForActiveTab();
          flash("Browser reset to defaults");
        }
      });
    }
  }

  function setupSegmentControl(id, onSelect) {
    var ctrl = $(id);
    if (!ctrl) return;
    var btns = ctrl.querySelectorAll(".segment-btn");
    btns.forEach(function (btn) {
      btn.addEventListener("click", function () {
        btns.forEach(function (b) { b.classList.remove("active"); });
        btn.classList.add("active");
        onSelect(btn.getAttribute("data-val"));
      });
    });
  }

  function renderNotFoundPage(container, url) {
    var wrap = document.createElement("div");
    wrap.style.cssText = "max-width:560px;margin:120px auto;text-align:center;padding:24px;";
    wrap.innerHTML =
      '<h1 style="font-family:var(--font-serif);font-size:32px;margin-bottom:12px;">Page Not Found</h1>' +
      '<p style="color:var(--muted);margin-bottom:24px;">The address <code>' + url + '</code> does not exist.</p>' +
      '<button class="primary-btn" id="btn-back-newtab">Go to New Tab</button>';
    container.appendChild(wrap);

    var backBtn = $("btn-back-newtab");
    if (backBtn) {
      backBtn.addEventListener("click", function () {
        navigateTo("rix://newtab");
      });
    }
  }


  function applyTheme(theme) {
    var t = theme === "light" ? "light" : "dark";
    document.documentElement.setAttribute("data-theme", t);
    if (shell) shell.setAttribute("data-theme", t);

    var host = window.rix || window.rixChrome;
    if (host && host.chrome && typeof host.chrome.setTheme === "function") {
      try { host.chrome.setTheme(t); } catch (e) {}
    }
  }

  function toggleTheme() {
    var next = state.settings.theme === "dark" ? "light" : "dark";
    state.settings.theme = next;
    applyTheme(next);
    saveState();
    flash("Theme: " + (next === "dark" ? "Dark" : "Light"));
  }

  function applySidebarMode(mode) {
    var m = mode === "rail" ? "rail" : "open";
    if (shell) shell.setAttribute("data-sidebar", m);
  }

  function toggleSidebar() {
    var next = state.settings.sidebar === "open" ? "rail" : "open";
    state.settings.sidebar = next;
    applySidebarMode(next);
    saveState();
  }


  var selectedSuggestionIndex = -1;
  var currentSuggestionItems = [];
  var originalTypedQuery = "";

  function openSuggestions(query) {
    if (!omniboxSuggestions || !suggestionsList) return;
    var q = (query || "").trim().toLowerCase();
    originalTypedQuery = query || "";

    suggestionsList.innerHTML = "";
    currentSuggestionItems = [];
    selectedSuggestionIndex = -1;
    var count = 0;

    if (q.length > 0) {
      var searchItem = document.createElement("div");
      searchItem.className = "suggestion-item";
      var searchUrl = getSearchEngineUrl(q);
      searchItem.innerHTML =
        '<span class="suggestion-icon"><svg class="ui-icon" viewBox="0 0 24 24"><path fill="currentColor" d="M15.5 14h-.79l-.28-.27A6.471 6.471 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"/></svg></span>' +
        '<div class="suggestion-content">' +
          '<span class="suggestion-title">Search Google for "' + escapeHtml(q) + '"</span>' +
          '<span class="suggestion-url">' + escapeHtml(searchUrl) + '</span>' +
        '</div>' +
        '<span class="suggestion-tag">Search</span>';

      searchItem.addEventListener("mousedown", function () {
        navigateTo(searchUrl);
      });
      suggestionsList.appendChild(searchItem);
      currentSuggestionItems.push({ element: searchItem, url: searchUrl });
      count++;
    }

    var builtinPages = [
      { title: "New Tab", url: "rix://newtab" },
      { title: "Bookmarks", url: "rix://bookmarks" },
      { title: "History", url: "rix://history" },
      { title: "Downloads", url: "rix://downloads" },
      { title: "Shortcuts", url: "rix://shortcuts" },
      { title: "Settings", url: "rix://settings" },
    ];

    builtinPages.forEach(function (page) {
      if (!q || page.title.toLowerCase().indexOf(q) !== -1 || page.url.indexOf(q) !== -1) {
        if (count < 8) {
          addSuggestionItem(page.title, page.url, "Page");
          count++;
        }
      }
    });

    state.bookmarks.forEach(function (b) {
      if (count < 8 && (!q || b.title.toLowerCase().indexOf(q) !== -1 || b.url.toLowerCase().indexOf(q) !== -1)) {
        addSuggestionItem(b.title, b.url, "Bookmark");
        count++;
      }
    });

    state.history.forEach(function (h) {
      if (count < 8 && (!q || h.title.toLowerCase().indexOf(q) !== -1 || h.url.toLowerCase().indexOf(q) !== -1)) {
        addSuggestionItem(h.title, h.url, "History");
        count++;
      }
    });

    if (count > 0) {
      omniboxSuggestions.removeAttribute("hidden");
    } else {
      omniboxSuggestions.setAttribute("hidden", "true");
    }
  }

  function addSuggestionItem(title, url, tag) {
    var item = document.createElement("div");
    item.className = "suggestion-item";
    item.innerHTML =
      '<span class="tab-item-monogram" style="width:20px;height:20px;font-size:10px;">' + getMonogram(title, url) + '</span>' +
      '<div class="suggestion-content">' +
        '<span class="suggestion-title">' + escapeHtml(title) + '</span>' +
        '<span class="suggestion-url">' + escapeHtml(url) + '</span>' +
      '</div>' +
      '<span class="suggestion-tag">' + tag + '</span>';

    item.addEventListener("mousedown", function () {
      navigateTo(url);
    });
    suggestionsList.appendChild(item);
    currentSuggestionItems.push({ element: item, url: url });
  }

  function closeSuggestions() {
    if (omniboxSuggestions) {
      omniboxSuggestions.setAttribute("hidden", "true");
    }
  }


  function openPalette() {
    if (!commandPalette || !paletteInput) return;
    commandPalette.removeAttribute("hidden");
    paletteInput.value = "";
    paletteInput.focus();
    renderPaletteResults("");
  }

  function closePalette() {
    if (commandPalette) {
      commandPalette.setAttribute("hidden", "true");
    }
  }

  function renderPaletteResults(query) {
    if (!paletteResults) return;
    paletteResults.innerHTML = "";

    var q = (query || "").trim().toLowerCase();

    var commands = [
      { id: "act_newtab", title: "New Tab", hint: "Ctrl+T", action: function () { createTab("rix://newtab", true); } },
      { id: "act_theme", title: "Toggle Theme (Dark / Light)", hint: "", action: toggleTheme },
      { id: "act_sidebar", title: "Toggle Sidebar (Expanded / Rail)", hint: "Ctrl+B", action: toggleSidebar },
      { id: "act_bookmarks", title: "Open Bookmarks", hint: "Ctrl+D", action: function () { navigateTo("rix://bookmarks"); } },
      { id: "act_history", title: "Open History", hint: "Ctrl+H", action: function () { navigateTo("rix://history"); } },
      { id: "act_downloads", title: "Open Downloads", hint: "Ctrl+J", action: function () { navigateTo("rix://downloads"); } },
      { id: "act_shortcuts", title: "Keyboard Shortcuts", hint: "Ctrl+/", action: function () { navigateTo("rix://shortcuts"); } },
      { id: "act_settings", title: "Browser Settings", hint: "Ctrl+,", action: function () { navigateTo("rix://settings"); } },
      { id: "act_clear_hist", title: "Clear Browsing History", hint: "", action: function () { state.history = []; saveState(); flash("History cleared"); } },
    ];

    var results = commands.filter(function (cmd) {
      return !q || cmd.title.toLowerCase().indexOf(q) !== -1;
    });

    state.tabs.forEach(function (tab) {
      if (!q || tab.title.toLowerCase().indexOf(q) !== -1 || tab.url.toLowerCase().indexOf(q) !== -1) {
        results.push({
          id: "tab_" + tab.id,
          title: "Switch to: " + tab.title,
          hint: tab.url,
          action: function () { switchTab(tab.id); },
        });
      }
    });

    results.forEach(function (res, index) {
      var item = document.createElement("div");
      item.className = "palette-item" + (index === 0 ? " active" : "");
      item.innerHTML =
        '<div class="palette-item-text">' + res.title + '</div>' +
        '<div class="palette-item-hint">' + res.hint + '</div>';

      item.addEventListener("click", function () {
        closePalette();
        res.action();
      });

      paletteResults.appendChild(item);
    });
  }


  function setupEventListeners() {
    var brandBtn = $("brand-home-btn");
    if (brandBtn) {
      brandBtn.addEventListener("click", function () {
        if (state.settings && state.settings.sidebar === "rail") {
          toggleSidebar();
        } else {
          navigateTo("rix://newtab");
        }
      });
    }

    var sidebarToggleBtn = $("sidebar-toggle-btn");
    if (sidebarToggleBtn) {
      sidebarToggleBtn.addEventListener("click", function () {
        toggleSidebar();
      });
    }

    var sidebarAddTabBtn = $("sidebar-add-tab-btn");
    if (sidebarAddTabBtn) sidebarAddTabBtn.addEventListener("click", function () { createTab("rix://newtab", true); });

    var securityLockBtn = $("security-lock-btn");
    var siteInfoPopup = $("site-info-popup");
    if (securityLockBtn && siteInfoPopup) {
      securityLockBtn.addEventListener("click", function (e) {
        e.stopPropagation();
        var isHidden = siteInfoPopup.hasAttribute("hidden");
        if (isHidden) {
          updateSiteInfoPopup();
          siteInfoPopup.removeAttribute("hidden");
        } else {
          siteInfoPopup.setAttribute("hidden", "true");
        }
      });
      window.addEventListener("click", function (e) {
        if (!siteInfoPopup.contains(e.target) && !securityLockBtn.contains(e.target)) {
          siteInfoPopup.setAttribute("hidden", "true");
        }
      });
    }

    var clearTabCookiesBtn = $("btn-clear-tab-cookies");
    if (clearTabCookiesBtn) {
      clearTabCookiesBtn.addEventListener("click", function (e) {
        e.stopPropagation();
        var tab = getActiveTab();
        if (!tab) return;
        var url = tab.url || "";
        var isInternal = isInternalUrl(url);
        if (isInternal) return;
        var domain = getDomain(url);
        var host = window.rix || window.rixChrome;
        if (host && host.cookies && typeof host.cookies.deleteForDomain === "function") {
          try { host.cookies.deleteForDomain(domain); } catch (e) {}
        }
        var countEl = $("site-cookie-count");
        if (countEl) countEl.textContent = "0";
        flash("Cleared cookies and data for " + domain);
        if (siteInfoPopup) siteInfoPopup.setAttribute("hidden", "true");
        if (host && host.navigation && typeof host.navigation.reload === "function") {
          try { host.navigation.reload(); } catch (e) {}
        }
      });
    }

    function updateSiteInfoPopup() {
      var tab = getActiveTab();
      var domainEl = $("site-info-domain");
      var statusEl = $("site-info-status");
      var countEl = $("site-cookie-count");
      var clearBtn = $("btn-clear-tab-cookies");
      if (!tab) return;
      var url = tab.url || "rix://newtab";
      var isInternal = isInternalUrl(url);
      var domain = isInternal ? url : getDomain(url);
      if (domainEl) domainEl.textContent = domain;
      if (statusEl) {
        statusEl.textContent = isInternal ? "Internal Browser Surface" : "Secure HTTPS Connection";
      }
      var host = window.rix || window.rixChrome;
      var cookiesCount = 0;
      if (!isInternal && host && host.cookies && typeof host.cookies.list === "function") {
        try {
          var list = host.cookies.list();
          var matched = list.find(function(c) {
            return c.domain === domain || domain.indexOf(c.domain) !== -1;
          });
          if (matched) cookiesCount = matched.count;
        } catch (e) {}
      }
      if (countEl) countEl.textContent = String(cookiesCount);
      if (clearBtn) {
        clearBtn.disabled = isInternal;
      }
    }

    var navBack = $("nav-back");
    if (navBack) {
      navBack.addEventListener("click", function () {
        var host = window.rix || window.rixChrome;
        if (host && host.navigation && typeof host.navigation.back === "function") {
          try { host.navigation.back(); } catch (e) {}
        }
      });
    }

    var navForward = $("nav-forward");
    if (navForward) {
      navForward.addEventListener("click", function () {
        var host = window.rix || window.rixChrome;
        if (host && host.navigation && typeof host.navigation.forward === "function") {
          try { host.navigation.forward(); } catch (e) {}
        }
      });
    }

    var navReload = $("nav-reload");
    if (navReload) {
      navReload.addEventListener("click", function () {
        var host = window.rix || window.rixChrome;
        if (host && host.navigation && typeof host.navigation.reload === "function") {
          try { host.navigation.reload(); } catch (e) {}
        } else {
          var tab = getActiveTab();
          if (tab) updateViewForActiveTab();
        }
      });
    }

    var navHome = $("nav-home");
    if (navHome) {
      navHome.addEventListener("click", function () {
        navigateTo("rix://newtab");
      });
    }

    if (omnibox) {
      omnibox.addEventListener("keydown", function (e) {
        if (e.key === "ArrowDown") {
          if (currentSuggestionItems.length > 0) {
            e.preventDefault();
            if (selectedSuggestionIndex >= 0 && selectedSuggestionIndex < currentSuggestionItems.length) {
              currentSuggestionItems[selectedSuggestionIndex].element.classList.remove("selected", "active");
            }
            selectedSuggestionIndex = (selectedSuggestionIndex + 1) % currentSuggestionItems.length;
            var cur = currentSuggestionItems[selectedSuggestionIndex];
            cur.element.classList.add("selected", "active");
            cur.element.scrollIntoView({ block: "nearest" });
            omnibox.value = cur.url;
          }
        } else if (e.key === "ArrowUp") {
          if (currentSuggestionItems.length > 0) {
            e.preventDefault();
            if (selectedSuggestionIndex >= 0 && selectedSuggestionIndex < currentSuggestionItems.length) {
              currentSuggestionItems[selectedSuggestionIndex].element.classList.remove("selected", "active");
            }
            if (selectedSuggestionIndex <= 0) {
              selectedSuggestionIndex = -1;
              omnibox.value = originalTypedQuery;
            } else {
              selectedSuggestionIndex = selectedSuggestionIndex - 1;
              var cur = currentSuggestionItems[selectedSuggestionIndex];
              cur.element.classList.add("selected", "active");
              cur.element.scrollIntoView({ block: "nearest" });
              omnibox.value = cur.url;
            }
          }
        } else if (e.key === "Enter") {
          if (selectedSuggestionIndex >= 0 && selectedSuggestionIndex < currentSuggestionItems.length) {
            navigateTo(currentSuggestionItems[selectedSuggestionIndex].url);
          } else {
            navigateTo(omnibox.value);
          }
          closeSuggestions();
          omnibox.blur();
        } else if (e.key === "Escape") {
          closeSuggestions();
          var tab = getActiveTab();
          if (tab) omnibox.value = tab.url;
          omnibox.blur();
        }
      });

      omnibox.addEventListener("input", function () {
        openSuggestions(omnibox.value);
      });

      omnibox.addEventListener("focus", function () {
        openSuggestions(omnibox.value);
        omnibox.select();
      });

      omnibox.addEventListener("blur", function () {
        setTimeout(closeSuggestions, 200);
      });
    }

    var paletteHint = $("omnibox-palette-hint");
    if (paletteHint) paletteHint.addEventListener("click", openPalette);

    if (btnStar) btnStar.addEventListener("click", toggleBookmarkCurrentTab);

    var themeToggle = $("theme-toggle");
    if (themeToggle) themeToggle.addEventListener("click", toggleTheme);

    var btnMore = $("btn-more");
    if (btnMore && moreMenu) {
      btnMore.addEventListener("click", function (e) {
        e.stopPropagation();
        var isHidden = moreMenu.hasAttribute("hidden");
        if (isHidden) {
          var activeTab = getActiveTab();
          var pinLabel = $("menu-pin-label");
          if (pinLabel && activeTab) {
            pinLabel.textContent = activeTab.pinned ? "Unpin Tab" : "Pin Tab";
          }
          moreMenu.removeAttribute("hidden");
        } else {
          moreMenu.setAttribute("hidden", "true");
        }
      });

      window.addEventListener("click", function () {
        moreMenu.setAttribute("hidden", "true");
      });
    }

    var menuActions = [
      { id: "menu-new-tab", fn: function () { createTab("rix://newtab", true); } },
      { id: "menu-duplicate", fn: function () { duplicateTab(state.activeTabId); } },
      { id: "menu-pin", fn: function () { togglePinTab(state.activeTabId); } },
      { id: "menu-copy-url", fn: function () {
        var tab = getActiveTab();
        if (tab && navigator.clipboard) {
          navigator.clipboard.writeText(tab.url);
          flash("URL copied to clipboard");
        }
      }},
      { id: "menu-bookmarks", fn: function () { navigateTo("rix://bookmarks"); } },
      { id: "menu-history", fn: function () { navigateTo("rix://history"); } },
      { id: "menu-downloads", fn: function () { navigateTo("rix://downloads"); } },
      { id: "menu-shortcuts", fn: function () { navigateTo("rix://shortcuts"); } },
      { id: "menu-settings", fn: function () { navigateTo("rix://settings"); } },
    ];

    menuActions.forEach(function (item) {
      var btn = $(item.id);
      if (btn) {
        btn.addEventListener("click", function () {
          if (moreMenu) moreMenu.setAttribute("hidden", "true");
          item.fn();
        });
      }
    });

    var railItems = [
      { id: "nav-btn-bookmarks", url: "rix://bookmarks" },
      { id: "nav-btn-history", url: "rix://history" },
      { id: "nav-btn-downloads", url: "rix://downloads" },
      { id: "nav-btn-shortcuts", url: "rix://shortcuts" },
      { id: "nav-btn-settings", url: "rix://settings" },
    ];

    railItems.forEach(function (item) {
      var btn = $(item.id);
      if (btn) {
        btn.addEventListener("click", function () {
          navigateTo(item.url);
        });
      }
    });

    var paletteBackdrop = $("palette-backdrop");
    if (paletteBackdrop) paletteBackdrop.addEventListener("click", closePalette);

    if (paletteInput) {
      paletteInput.addEventListener("input", function () {
        renderPaletteResults(paletteInput.value);
      });

      paletteInput.addEventListener("keydown", function (e) {
        if (e.key === "Escape") {
          closePalette();
        } else if (e.key === "Enter") {
          var activeItem = paletteResults ? paletteResults.querySelector(".palette-item.active") : null;
          if (activeItem) activeItem.click();
        } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          var items = paletteResults ? paletteResults.querySelectorAll(".palette-item") : [];
          if (items.length === 0) return;
          var curIdx = -1;
          for (var i = 0; i < items.length; i++) {
            if (items[i].classList.contains("active")) { curIdx = i; break; }
          }
          if (curIdx !== -1) items[curIdx].classList.remove("active");
          if (e.key === "ArrowDown") {
            curIdx = (curIdx + 1) % items.length;
          } else {
            curIdx = (curIdx - 1 + items.length) % items.length;
          }
          items[curIdx].classList.add("active");
          items[curIdx].scrollIntoView({ block: "nearest" });
        }
      });
    }

    setupContextMenuEvents();

    window.addEventListener("keydown", function (e) {
      var isMac = navigator.platform.toUpperCase().indexOf("MAC") >= 0;
      var ctrl = isMac ? e.metaKey : e.ctrlKey;
      var key = e.key.toLowerCase();

      if (e.key === "Escape") {
        closePalette();
        closeSuggestions();
        hideContextMenu();
        if (moreMenu) moreMenu.setAttribute("hidden", "true");
        return;
      }

      if (ctrl && key === "k") {
        e.preventDefault();
        openPalette();
        return;
      }

      if (ctrl && key === "t" && !e.shiftKey) {
        e.preventDefault();
        createTab("rix://newtab", true);
        return;
      }

      if (ctrl && key === "w") {
        e.preventDefault();
        var cur = getActiveTab();
        if (cur) closeTab(cur.id);
        return;
      }

      if (ctrl && e.shiftKey && key === "t") {
        e.preventDefault();
        reopenClosedTab();
        return;
      }

      if (ctrl && key === "b") {
        e.preventDefault();
        toggleSidebar();
        return;
      }

      if (ctrl && key === "l") {
        e.preventDefault();
        if (omnibox) {
          omnibox.focus();
          omnibox.select();
        }
        return;
      }

      if (ctrl && key === "d") {
        e.preventDefault();
        toggleBookmarkCurrentTab();
        return;
      }

      if (ctrl && key === "h") {
        e.preventDefault();
        navigateTo("rix://history");
        return;
      }

      if (ctrl && key === "j") {
        e.preventDefault();
        navigateTo("rix://downloads");
        return;
      }

      if (ctrl && (key === "/" || key === "?")) {
        e.preventDefault();
        navigateTo("rix://shortcuts");
        return;
      }

      if (ctrl && key === ",") {
        e.preventDefault();
        navigateTo("rix://settings");
        return;
      }
    });
  }


  function init() {
    applyTheme(state.settings.theme || "dark");
    applySidebarMode(state.settings.sidebar || "open");
    if (shell) shell.setAttribute("data-density", state.settings.density || "regular");

    renderTabs();
    updateViewForActiveTab();
    setupEventListeners();
  }

  window.rixChrome = window.rixChrome || {};
  window.rixChrome.onTabUpdated = function (info) {
    if (!info) return;
    if (typeof info.activeTabId === "number") {
      state.activeTabId = info.activeTabId;
      if (Array.isArray(info.tabs)) {
        info.tabs.forEach(function (ht) {
          var cleanUrl = ht.url ? ht.url : "rix://newtab";
          var tab = state.tabs.find(function (t) { return t.id === ht.id; });
          if (tab) {
            tab.url = cleanUrl;
            if (ht.title) tab.title = ht.title;
          } else {
            state.tabs.push({
              id: ht.id,
              url: cleanUrl,
              title: ht.title || "New Tab",
              pinned: false,
            });
          }
        });
      }
      renderTabs();
      updateViewForActiveTab();
    }
  };
  window.onTabUpdated = window.rixChrome.onTabUpdated;
  window.rixChrome.navigateTo = navigateTo;
  window.navigateTo = navigateTo;
  window.rixChrome.createTab = createTab;
  window.createTab = createTab;

  init();
})();
