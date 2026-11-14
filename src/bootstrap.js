(function () {
  if (window.__rsdocBooted) { return; }
  window.__rsdocBooted = true;

  function detectRoot() {
    var refs = document.querySelectorAll(
      'link[href*="static.files"], script[src*="static.files"]'
    );
    for (var i = 0; i < refs.length; i++) {
      var u = refs[i].getAttribute("href") || refs[i].getAttribute("src") || "";
      var at = u.indexOf("static.files");
      if (at > 0) { return u.slice(0, at); }
    }

    var css = document.querySelector('link[rel="stylesheet"][href*="rustdoc"]');
    if (css) {
      var h = css.getAttribute("href") || "";
      var slash = h.lastIndexOf("/");
      if (slash >= 0) { return h.slice(0, slash + 1); }
    }

    var src = document.querySelector('a.src[href*="/src/"]');
    if (src) {
      var s = src.getAttribute("href") || "";
      var at2 = s.indexOf("src/");
      if (at2 > 0) { return s.slice(0, at2); }
    }

    return "";
  }

  // Candidate prefixes, best guess first, then the common depths as a net.
  function prefixesFor(root) {
    var list = [];
    if (root) { list.push(root); }
    var fallbacks = ["../", "../../", "../../../", "./"];
    for (var i = 0; i < fallbacks.length; i++) {
      if (list.indexOf(fallbacks[i]) < 0) { list.push(fallbacks[i]); }
    }
    return list;
  }

  var prefixes = prefixesFor(detectRoot());

  function loadMermaid() {
    var theme = localStorage.getItem("rustdoc-theme");
    var dark = (theme === "dark" || theme === "ayu");
    var i = 0;

    function attempt() {
      if (i >= prefixes.length) {
        console.error("rsdoc: mermaid.min.js not found from " + location.pathname);
        return;
      }
      var s = document.createElement("script");
      s.src = prefixes[i++] + "mermaid.min.js";
      s.onload = function () {
        window.mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default" });
        window.mermaid.run();
      };
      s.onerror = attempt;
      document.head.appendChild(s);
    }

    attempt();
  }

  function bindImage(el) {
    var rel = el.getAttribute("data-rsdoc-src");
    var i = 0;

    function attempt() {
      if (i >= prefixes.length) { return; }
      el.src = prefixes[i++] + rel;
    }

    el.onerror = attempt;
    attempt();
  }

  function resolve() {
    // Wide diagrams used to push the rustdoc content area sideways, producing a
    // horizontal scrollbar. Constrain both diagrams and referenced images to the
    // column width; `height:auto` keeps the aspect ratio.
    var style = document.createElement("style");
    style.textContent =
      "pre.mermaid svg{max-width:100%;height:auto;}" +
      "img[data-rsdoc-src]{max-width:100%;height:auto;}";
    document.head.appendChild(style);

    var imgs = document.querySelectorAll("img[data-rsdoc-src]");
    for (var i = 0; i < imgs.length; i++) { bindImage(imgs[i]); }

    var links = document.querySelectorAll("[data-rsdoc-src]:not(img)");
    for (var j = 0; j < links.length; j++) {
      links[j].setAttribute(
        "href",
        prefixes[0] + links[j].getAttribute("data-rsdoc-src")
      );
    }

    if (document.querySelector("pre.mermaid")) { loadMermaid(); }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", resolve);
  } else {
    resolve();
  }
})();
