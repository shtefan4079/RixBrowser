
(function () {
  "use strict";

  var state = {
    tabs: [{ id: 1, url: "about:blank", title: "New Tab", isLoading: false }],
    activeTabId: 1,
    nextTabId: 2,
    activeLoading: false,
    closedTabs: [],
  };

  var renderedTabs = Object.create(null);
  var tabMarkupInitialized = false;
  var omniboxEditing = false;

  function rix() {
    return typeof window.rix !== "undefined" ? window.rix : null;
  }

  function $(id) {
    return document.getElementById(id);
  }

  function tabId(value) {
    var id = Number(value);
    if (!isFinite(id) || id < 0 || Math.floor(id) !== id) return null;
    return id;
  }

  function setAttributeIfChanged(element, name, value) {
    if (element.getAttribute(name) !== value) element.setAttribute(name, value);
  }

  function codeIs(event, code, key) {
    var actualCode = String(event.code || "");
    if (actualCode) return actualCode === code;
    return String(event.key || "").toLowerCase() === String(key || "").toLowerCase();
  }

  function digitFromEvent(event) {
    var code = String(event.code || "");
    if (/^Digit[1-9]$/.test(code)) return Number(code.slice(5));
    var key = String(event.key || "");
    if (/^[1-9]$/.test(key)) return Number(key);
    return 0;
  }

  var omnibox = $("omnibox");
  var tabBar = $("tab-bar");
  var newTabBtn = $("new-tab-btn");
  var navBack = $("nav-back");
  var navForward = $("nav-forward");
  var navReload = $("nav-reload");
  var navHome = $("nav-home");
  var themeToggle = $("theme-toggle");
  var orientationToggle = $("tabs-orientation-toggle");
  var helpToggle = $("help-toggle");
  var shortcutsPanel = $("shortcuts-panel");
  var shortcutsClose = $("shortcuts-close");
  var loadProgress = $("load-progress");
  var shell = $("browser-shell");
  var lockBadge = $("security-lock");
  var secBadge = $("security-badge");
  var crashOverlay = $("crash-overlay");
  var crashReload = $("crash-reload");

  var iconPaths = {
    page: "M5 3h9l5 5v13H5V3Zm2 2v14h10V9h-5V5H7Zm7 .8V7h1.2L14 5.8Z",
    close: "m6.4 5 5.6 5.6L17.6 5 19 6.4 13.4 12l5.6 5.6-1.4 1.4-5.6-5.6L6.4 19 5 17.6l5.6-5.6L5 6.4 6.4 5Z",
    lock: "M7 10V7a5 5 0 0 1 10 0v3h2v12H5V10h2Zm2 0h6V7a3 3 0 0 0-6 0v3Zm-2 2v8h10v-8H7Zm4 2h2v4h-2v-4Z",
    warning: "M11 3h2l10 18H1L11 3Zm1 3L4.4 19h15.2L12 6Zm-1 4h2v5h-2v-5Zm0 6h2v2h-2v-2Z",
    info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18Zm0 2a7 7 0 1 1 0 14 7 7 0 0 1 0-14Zm-1 5h2v6h-2v-6Zm0-3h2v2h-2V7Z",
  };

  function iconMarkup(name) {
    return '<svg class="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="' + iconPaths[name] + '" /></svg>';
  }

  function createTabNode(id) {
    var el = document.createElement("div");
    el.setAttribute("role", "tab");
    el.setAttribute("data-tab-id", String(id));

    var favicon = document.createElement("span");
    favicon.className = "tab-favicon";
    favicon.setAttribute("aria-hidden", "true");
    favicon.innerHTML = iconMarkup("page");

    var title = document.createElement("span");
    title.className = "tab-title";

    var loading = document.createElement("span");
    loading.className = "tab-loading";
    loading.setAttribute("aria-hidden", "true");

    var close = document.createElement("button");
    close.className = "tab-close";
    close.setAttribute("title", "Close tab (Ctrl+W)");
    close.innerHTML = iconMarkup("close");
    close.addEventListener("click", function (ev) {
      ev.stopPropagation();
      closeTab(id);
    });

    el.appendChild(favicon);
    el.appendChild(title);
    el.appendChild(loading);
    el.appendChild(close);
    el.addEventListener("click", function () {
      switchTab(id);
    });
    el.addEventListener("keydown", function (ev) {
      if (codeIs(ev, "Enter", "Enter") || codeIs(ev, "Space", " ")) {
        ev.preventDefault();
        switchTab(id);
        return;
      }
      var direction = codeIs(ev, "ArrowLeft", "ArrowLeft") || codeIs(ev, "ArrowUp", "ArrowUp") ? -1
        : (codeIs(ev, "ArrowRight", "ArrowRight") || codeIs(ev, "ArrowDown", "ArrowDown") ? 1 : 0);
      var home = codeIs(ev, "Home", "Home");
      var end = codeIs(ev, "End", "End");
      if (!direction && !home && !end) return;
      ev.preventDefault();
      var index = 0;
      for (var i = 0; i < state.tabs.length; i++) {
        if (state.tabs[i].id === id) index = i;
      }
      var target = home ? 0 : (end ? state.tabs.length - 1
        : (index + direction + state.tabs.length) % state.tabs.length);
      var nextId = state.tabs[target].id;
      switchTab(nextId);
      focusTab(nextId);
    });
    return { el: el, title: title, close: close };
  }

  function focusTab(id) {
    var node = renderedTabs[id];
    if (node && typeof node.el.focus === "function") node.el.focus();
  }

  function renderTabs() {
    if (!tabBar) return;
    if (!tabMarkupInitialized) {
      tabMarkupInitialized = true;
      var initialTabs = tabBar.querySelectorAll(".tab");
      for (var i = 0; i < initialTabs.length; i++) tabBar.removeChild(initialTabs[i]);
    }
    var liveIds = Object.create(null);
    state.tabs.forEach(function (tab) {
      liveIds[tab.id] = true;
    });
    Object.keys(renderedTabs).forEach(function (id) {
      if (!liveIds[id]) {
        tabBar.removeChild(renderedTabs[id].el);
        delete renderedTabs[id];
      }
    });
    state.tabs.forEach(function (tab, index) {
      var node = renderedTabs[tab.id];
      if (!node) {
        node = createTabNode(tab.id);
        renderedTabs[tab.id] = node;
      }
      var active = tab.id === state.activeTabId;
      var title = tab.title || "New Tab";
      var className = "tab" + (active ? " active" : "") + (tab.isLoading ? " is-loading" : "");
      setAttributeIfChanged(node.el, "class", className);
      setAttributeIfChanged(node.el, "tabindex", active ? "0" : "-1");
      setAttributeIfChanged(node.el, "aria-selected", active ? "true" : "false");
      setAttributeIfChanged(node.el, "aria-busy", tab.isLoading ? "true" : "false");
      setAttributeIfChanged(node.el, "title", title);
      setAttributeIfChanged(node.close, "aria-label", "Close " + title);
      if (node.title.textContent !== title) node.title.textContent = title;
      var atIndex = tabBar.querySelectorAll(".tab")[index];
      if (!atIndex) tabBar.appendChild(node.el);
      else if (atIndex.getAttribute("data-tab-id") !== String(tab.id)) {
        tabBar.insertBefore(node.el, atIndex);
      }
    });
  }

  function findTab(id) {
    for (var i = 0; i < state.tabs.length; i++) {
      if (state.tabs[i].id === id) return state.tabs[i];
    }
    return null;
  }

  function activeTab() {
    return findTab(state.activeTabId) || state.tabs[0] || null;
  }

  function updateLoadingIndicator(explicit) {
    var tab = activeTab();
    var loading = typeof explicit === "boolean" ? explicit : !!(tab && tab.isLoading);
    state.activeLoading = loading;
    if (loadProgress) {
      if (loading) loadProgress.removeAttribute("hidden");
      else loadProgress.setAttribute("hidden", "");
    }
    if (shell && shell.classList) {
      if (loading) shell.classList.add("has-loading");
      else shell.classList.remove("has-loading");
    }
  }

  function updateSecurityBadge(url) {
    var tab = activeTab();
    var current = url || (tab && tab.url) || "about:blank";
    var secure = /^https:/i.test(current);
    var insecure = /^http:/i.test(current);
    if (secBadge) secBadge.textContent = secure ? "HTTPS" : (insecure ? "Not secure" : "Internal");
    if (lockBadge) {
      var icon = secure ? "lock" : (insecure ? "warning" : "info");
      if (lockBadge.getAttribute("data-icon") !== icon) {
        lockBadge.innerHTML = iconMarkup(icon);
        lockBadge.setAttribute("data-icon", icon);
      }
      lockBadge.className = "omnibox-lock" + (secure ? " is-secure" : (insecure ? " is-insecure" : ""));
    }
  }

  function setAddressBarFromActive(force) {
    var tab = activeTab();
    if (!tab) return;
    if (omnibox && (force || !omniboxEditing)) {
      omnibox.value = tab.url === "about:blank" ? "" : (tab.url || "");
    }
    updateSecurityBadge(tab.url || "about:blank");
    updateLoadingIndicator();
  }

  function replaceTabsFromHost(hostTabs, activeHint, activeLoadingHint) {
    if (!hostTabs || typeof hostTabs.length !== "number") return false;
    var nextTabs = [];
    var nextActiveId = tabId(activeHint);
    var maxId = 0;

    for (var i = 0; i < hostTabs.length; i++) {
      var descriptor = hostTabs[i];
      if (!descriptor || typeof descriptor !== "object") continue;
      var id = tabId(descriptor.id);
      if (id === null) continue;
      var previous = findTab(id);
      var tab = {
        id: id,
        url: descriptor.url == null
          ? (previous ? previous.url : "about:blank")
          : String(descriptor.url),
        title: descriptor.title == null
          ? (previous ? previous.title : "New Tab")
          : String(descriptor.title),
        isLoading: descriptor.isLoading === true || descriptor.is_loading === true,
      };
      nextTabs.push(tab);
      if (id > maxId) maxId = id;
      if (descriptor.isActive === true || descriptor.active === true) nextActiveId = id;
    }

    if (nextTabs.length === 0) return false;
    var activeIsPresent = false;
    for (var j = 0; j < nextTabs.length; j++) {
      if (nextTabs[j].id === nextActiveId) activeIsPresent = true;
    }
    state.tabs = nextTabs;
    state.activeTabId = activeIsPresent ? nextActiveId : nextTabs[0].id;
    state.nextTabId = Math.max(state.nextTabId, maxId + 1);
    renderTabs();
    setAddressBarFromActive(false);
    updateLoadingIndicator(typeof activeLoadingHint === "boolean" ? activeLoadingHint : null);
    return true;
  }

  function syncTabsFromHost() {
    var host = rix();
    if (!host || !host.tabs || typeof host.tabs.list !== "function") return false;
    try {
      return replaceTabsFromHost(host.tabs.list(), state.activeTabId, null);
    } catch (err) {
      return false;
    }
  }

  function createTab(url, focusOmniboxAfter) {
    var target = url || "https://www.youtube.com/";
    var focusAddress = focusOmniboxAfter !== false;
    var host = rix();
    if (host && host.tabs && typeof host.tabs.create === "function") {
      var createdId = null;
      try {
        createdId = tabId(host.tabs.create({ url: target }));
      } catch (err) {
        createdId = null;
      }
      if (createdId !== null) {
        if (!syncTabsFromHost()) {
          state.tabs.push({ id: createdId, url: target, title: "New Tab", isLoading: false });
          state.activeTabId = createdId;
          state.nextTabId = Math.max(state.nextTabId, createdId + 1);
          renderTabs();
          setAddressBarFromActive(true);
        }
        if (focusAddress) focusOmnibox(true);
        return createdId;
      }
    }
    var tab = { id: state.nextTabId++, url: target, title: "New Tab", isLoading: false };
    state.tabs.push(tab);
    state.activeTabId = tab.id;
    renderTabs();
    setAddressBarFromActive(true);
    if (focusAddress) focusOmnibox(true);
    return tab.id;
  }

  function rememberClosedTab(tab) {
    if (!tab) return;
    state.closedTabs.push({
      url: tab.url || "about:blank",
      title: tab.title || "New Tab",
    });
    if (state.closedTabs.length > 25) state.closedTabs.shift();
  }

  function reopenClosedTab() {
    if (state.closedTabs.length === 0) return false;
    var tab = state.closedTabs.pop();
    return createTab(tab.url, false) !== null;
  }

  function closeTab(id) {
    id = tabId(id);
    var tab = id === null ? null : findTab(id);
    if (!tab) return false;
    var node = renderedTabs[id];
    var activeElement = document.activeElement;
    var focusInTabs = !!(node && activeElement && node.el.contains(activeElement));
    var host = rix();
    if (host && host.tabs && typeof host.tabs.close === "function") {
      var closed = false;
      try {
        closed = host.tabs.close(id);
      } catch (err) {
        return false;
      }
      if (closed === false) return false;
      rememberClosedTab(tab);
      if (!syncTabsFromHost()) {
        if (state.tabs.length > 1) {
          state.tabs = state.tabs.filter(function (item) { return item.id !== id; });
          if (state.activeTabId === id) state.activeTabId = state.tabs[state.tabs.length - 1].id;
        }
        renderTabs();
        setAddressBarFromActive(true);
      }
      if (focusInTabs) focusTab(state.activeTabId);
      return true;
    }
    if (state.tabs.length <= 1) return false;
    rememberClosedTab(tab);
    state.tabs = state.tabs.filter(function (item) { return item.id !== id; });
    if (state.activeTabId === id) {
      state.activeTabId = state.tabs[state.tabs.length - 1].id;
    }
    renderTabs();
    setAddressBarFromActive(true);
    if (focusInTabs) focusTab(state.activeTabId);
    return true;
  }

  function switchTab(id) {
    id = tabId(id);
    if (id === null || !findTab(id)) return false;
    var host = rix();
    if (host && host.tabs) {
      var switched = true;
      try {
        if (typeof host.tabs.switch === "function") {
          switched = host.tabs.switch(id);
        } else if (typeof host.tabs.select === "function") {
          switched = host.tabs.select(id);
        }
      } catch (err) {
        return false;
      }
      if (switched === false) return false;
      if (syncTabsFromHost()) {
        omniboxEditing = false;
        setAddressBarFromActive(true);
        return true;
      }
    }
    state.activeTabId = id;
    omniboxEditing = false;
    renderTabs();
    setAddressBarFromActive(true);
    return true;
  }

  function onTabUpdated(info) {
    if (!info || typeof info !== "object") return false;
    if (info.tabs && replaceTabsFromHost(info.tabs, info.activeTabId, info.activeLoading)) return true;
    if (info.tab && typeof info.tab === "object") {
      var outer = info;
      var nested = info.tab;
      info = {
        id: nested.id,
        url: nested.url,
        title: nested.title,
        isActive: nested.isActive,
        isLoading: nested.isLoading,
        activeTabId: typeof outer.activeTabId !== "undefined" ? outer.activeTabId : nested.activeTabId,
        activeLoading: typeof outer.activeLoading !== "undefined" ? outer.activeLoading : nested.activeLoading,
      };
    }
    var id = tabId(info.id);
    if (id === null) return false;
    var tab = findTab(id);
    if (!tab) {
      tab = { id: id, url: "about:blank", title: "New Tab", isLoading: false };
      state.tabs.push(tab);
    }
    if (typeof info.url !== "undefined" && info.url !== null) tab.url = String(info.url);
    if (typeof info.title !== "undefined" && info.title !== null) tab.title = String(info.title);
    if (typeof info.isLoading !== "undefined") tab.isLoading = info.isLoading === true;
    else if (typeof info.is_loading !== "undefined") tab.isLoading = info.is_loading === true;
    if (typeof info.activeTabId !== "undefined") {
      var activeId = tabId(info.activeTabId);
      if (activeId !== null && (findTab(activeId) || activeId === id)) state.activeTabId = activeId;
    } else if (info.isActive === true || info.active === true) {
      state.activeTabId = id;
    }
    state.nextTabId = Math.max(state.nextTabId, id + 1);
    if (!findTab(state.activeTabId) && !syncTabsFromHost()) state.activeTabId = id;
    renderTabs();
    setAddressBarFromActive(false);
    updateLoadingIndicator(typeof info.activeLoading === "boolean" ? info.activeLoading : null);
    return true;
  }

  function normalizeInput(query) {
    var text = (query || "").trim();
    if (!text) return null;
    return text;
  }

  function navigateTo(raw) {
    var url = normalizeInput(raw);
    if (!url) return null;
    var host = rix();
    if (host && host.navigation && typeof host.navigation.goTo === "function") {
      try {
        host.navigation.goTo(url);
      } catch (err) {
        return null;
      }
    }
    var tab = activeTab();
    if (tab) tab.url = url;
    omniboxEditing = false;
    if (omnibox) omnibox.value = url;
    updateSecurityBadge(url);
    return url;
  }

  function callNavigation(method) {
    var host = rix();
    var navigation = host && host.navigation;
    if (!navigation || typeof navigation[method] !== "function") return false;
    try {
      navigation[method]();
      return true;
    } catch (err) {
      return false;
    }
  }

  function focusOmnibox(selectText) {
    if (!omnibox) return;
    omniboxEditing = true;
    try {
      if (typeof omnibox.focus === "function") omnibox.focus();
      if (selectText && typeof omnibox.select === "function") omnibox.select();
    } catch (err) {
      return;
    }
  }

  function cycleTab(delta) {
    if (state.tabs.length < 2) return;
    var currentIndex = 0;
    for (var i = 0; i < state.tabs.length; i++) {
      if (state.tabs[i].id === state.activeTabId) currentIndex = i;
    }
    var nextIndex = (currentIndex + delta + state.tabs.length) % state.tabs.length;
    switchTab(state.tabs[nextIndex].id);
  }

  function selectTabByNumber(number) {
    if (!number) return;
    var index = number === 9 ? state.tabs.length - 1 : number - 1;
    if (index < 0 || index >= state.tabs.length) return;
    switchTab(state.tabs[index].id);
  }

  function shortcutsOpen() {
    return !!(shortcutsPanel && !shortcutsPanel.hasAttribute("hidden"));
  }

  function toggleShortcuts(force) {
    if (!shortcutsPanel || !helpToggle) return false;
    var open = typeof force === "boolean" ? force : !shortcutsOpen();
    if (open) shortcutsPanel.removeAttribute("hidden");
    else shortcutsPanel.setAttribute("hidden", "");
    helpToggle.setAttribute("aria-expanded", open ? "true" : "false");
    if (open) {
      if (shortcutsClose && typeof shortcutsClose.focus === "function") shortcutsClose.focus();
    } else if (typeof helpToggle.focus === "function") {
      helpToggle.focus();
    }
    return open;
  }

  function handleOmniboxKey(event) {
    if (codeIs(event, "Enter", "Enter") && event.altKey) {
      event.preventDefault();
      var altTarget = normalizeInput(omnibox.value);
      if (altTarget) createTab(altTarget, false);
      return;
    }
    if (codeIs(event, "Enter", "Enter")) {
      event.preventDefault();
      var destination = navigateTo(omnibox.value);
      if (destination) {
        if (typeof omnibox.blur === "function") omnibox.blur();
        omniboxEditing = false;
      }
    } else if (codeIs(event, "Escape", "Escape")) {
      event.preventDefault();
      setAddressBarFromActive(true);
      omniboxEditing = false;
      if (typeof omnibox.blur === "function") omnibox.blur();
    }
  }

  if (omnibox) {
    omnibox.addEventListener("keydown", handleOmniboxKey);
    omnibox.addEventListener("focus", function () {
      omniboxEditing = true;
    });
    omnibox.addEventListener("blur", function () {
      omniboxEditing = false;
      setAddressBarFromActive(true);
    });
    omnibox.addEventListener("input", function () {
      omniboxEditing = true;
    });
  }

  if (newTabBtn) {
    newTabBtn.addEventListener("click", function () {
      createTab("https://www.youtube.com/", true);
    });
  }
  if (navBack) {
    navBack.addEventListener("click", function () {
      callNavigation("back");
    });
  }
  if (navForward) {
    navForward.addEventListener("click", function () {
      callNavigation("forward");
    });
  }
  if (navReload) {
    navReload.addEventListener("click", function () {
      callNavigation("reload");
    });
  }
  if (navHome) {
    navHome.addEventListener("click", function () {
      navigateTo("https://www.youtube.com/");
    });
  }
  if (helpToggle) {
    helpToggle.addEventListener("click", function () {
      toggleShortcuts();
    });
  }
  if (shortcutsClose) {
    shortcutsClose.addEventListener("click", function () {
      toggleShortcuts(false);
    });
  }

  function currentTheme() {
    var root = document.documentElement;
    return (root && root.getAttribute && root.getAttribute("data-theme")) || "dark";
  }

  function updateChromeToggleLabels() {
    if (themeToggle) {
      var themeLabel = currentTheme() === "dark" ? "Use light theme" : "Use dark theme";
      themeToggle.setAttribute("aria-label", themeLabel);
      themeToggle.setAttribute("title", themeLabel);
    }
    if (orientationToggle && shell) {
      var vertical = shell.classList && shell.classList.contains("vertical-tabs");
      orientationToggle.setAttribute("aria-pressed", vertical ? "true" : "false");
      orientationToggle.setAttribute("title", vertical ? "Use horizontal tabs" : "Use vertical tabs");
    }
  }

  if (themeToggle) {
    themeToggle.addEventListener("click", function () {
      var next = currentTheme() === "dark" ? "light" : "dark";
      document.documentElement.setAttribute("data-theme", next);
      updateChromeToggleLabels();
      var host = rix();
      if (host && host.chrome && typeof host.chrome.setTheme === "function") {
        try {
          host.chrome.setTheme(next);
        } catch (err) {
          return;
        }
      }
    });
  }

  if (orientationToggle && shell) {
    orientationToggle.addEventListener("click", function () {
      shell.classList.toggle("vertical-tabs");
      updateChromeToggleLabels();
      var host = rix();
      if (host && host.chrome && typeof host.chrome.setTabOrientation === "function") {
        var vertical = shell.classList && shell.classList.contains("vertical-tabs");
        try {
          host.chrome.setTabOrientation(vertical ? "vertical" : "horizontal");
        } catch (err) {
          return;
        }
      }
    });
  }

  function setCrashOverlayVisible(visible) {
    if (!crashOverlay) return;
    if (visible) crashOverlay.removeAttribute("hidden");
    else crashOverlay.setAttribute("hidden", "");
  }

  if (crashReload) {
    crashReload.addEventListener("click", function () {
      callNavigation("reload");
      setCrashOverlayVisible(false);
    });
  }

  document.addEventListener("keydown", function (event) {
    if (event.defaultPrevented) return;
    var mod = event.ctrlKey || event.metaKey;
    var shift = event.shiftKey === true;
    var alt = event.altKey === true;

    if (alt && !mod && codeIs(event, "ArrowLeft", "ArrowLeft")) {
      event.preventDefault();
      callNavigation("back");
      return;
    }
    if (alt && !mod && codeIs(event, "ArrowRight", "ArrowRight")) {
      event.preventDefault();
      callNavigation("forward");
      return;
    }
    if (alt && !mod && codeIs(event, "Home", "Home")) {
      event.preventDefault();
      navigateTo("https://www.youtube.com/");
      return;
    }
    if (alt && !mod && codeIs(event, "KeyD", "d")) {
      event.preventDefault();
      focusOmnibox(true);
      return;
    }
    if (!mod && !alt && codeIs(event, "F6", "F6")) {
      event.preventDefault();
      focusOmnibox(true);
      return;
    }
    if (!mod && !alt && codeIs(event, "F1", "F1")) {
      event.preventDefault();
      toggleShortcuts();
      return;
    }
    if (codeIs(event, "Escape", "Escape")) {
      if (shortcutsOpen()) {
        event.preventDefault();
        toggleShortcuts(false);
      }
      return;
    }
    if (!mod || alt) return;

    if (codeIs(event, "Slash", "/")) {
      event.preventDefault();
      toggleShortcuts();
      return;
    }
    if (codeIs(event, "KeyT", "t")) {
      event.preventDefault();
      if (shift) reopenClosedTab();
      else createTab("https://www.youtube.com/", true);
      return;
    }
    if (codeIs(event, "KeyW", "w")) {
      event.preventDefault();
      closeTab(state.activeTabId);
      return;
    }
    if (codeIs(event, "KeyR", "r") || codeIs(event, "F5", "F5")) {
      event.preventDefault();
      callNavigation("reload");
      return;
    }
    if (codeIs(event, "KeyL", "l") || codeIs(event, "KeyK", "k")) {
      event.preventDefault();
      focusOmnibox(true);
      return;
    }
    if (codeIs(event, "Tab", "Tab")) {
      event.preventDefault();
      cycleTab(shift ? -1 : 1);
      return;
    }
    if (codeIs(event, "PageDown", "PageDown")) {
      event.preventDefault();
      cycleTab(1);
      return;
    }
    if (codeIs(event, "PageUp", "PageUp")) {
      event.preventDefault();
      cycleTab(-1);
      return;
    }
    var number = digitFromEvent(event);
    if (number) {
      event.preventDefault();
      selectTabByNumber(number);
    }
  });

  window.rixChrome = {
    onTabUpdated: onTabUpdated,
    setCrashOverlayVisible: setCrashOverlayVisible,
    updateSecurityBadge: updateSecurityBadge,
    toggleShortcuts: toggleShortcuts,
    getState: function () {
      return {
        tabs: state.tabs.map(function (tab) {
          return { id: tab.id, url: tab.url, title: tab.title, isLoading: tab.isLoading };
        }),
        activeTabId: state.activeTabId,
        activeLoading: state.activeLoading,
      };
    },
  };

  updateChromeToggleLabels();
  renderTabs();
  if (!syncTabsFromHost()) {
    renderTabs();
    setAddressBarFromActive(true);
  }
})();
