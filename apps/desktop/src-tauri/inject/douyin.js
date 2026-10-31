// 注入抖音側欄的腳本：讀取頁面自己請求的拉流網址，交給 SupaLive。
// 不代替頁面發請求，登入與簽名都由抖音頁面自己完成。唯一改動頁面的地方是開新分頁：
// 側欄只有一個頁面，新分頁一律改成在本頁打開。
(function () {
  if (window.__supaliveSniffer) return;
  window.__supaliveSniffer = true;

  function go(url) {
    try { location.href = new URL(String(url), location.href).href; } catch (e) {}
  }
  // window.open(網址) 直接在本頁打開；先開空白視窗、之後再設網址的寫法，回傳一個假視窗接住那個網址
  window.open = function (url) {
    if (url && String(url) !== "about:blank") { go(url); return window; }
    var loc = { assign: go, replace: go };
    Object.defineProperty(loc, "href", { get: function () { return "about:blank"; }, set: go });
    var fake = { closed: false, opener: null, close: function () {}, focus: function () {}, blur: function () {},
      document: { write: function () {}, writeln: function () {}, open: function () {}, close: function () {} } };
    Object.defineProperty(fake, "location", { get: function () { return loc; }, set: go });
    return fake;
  };
  // <a target="_blank"> 改成在本頁打開
  document.addEventListener("click", function (e) {
    var a = e.target && e.target.closest && e.target.closest("a[target]");
    if (a && a.target !== "_self" && a.href) a.target = "_self";
  }, true);

  var seen = {};
  var STREAM = /^https?:\/\/[^\s"'\\]+?\.(flv|m3u8)(\?[^\s"'\\]*)?$/i;
  var CDN = /douyincdn|douyinliving|douyinvod|bytecdn|byteimg|pull-|zjcdn|amemv/i;

  function webRid() {
    var m = location.pathname.match(/^\/(\d{4,})/);
    return m ? m[1] : null;
  }

  function anchorName() {
    var t = document.title || "";
    // 例：「小滿在唱歌的抖音直播间 - 抖音直播」
    t = t.replace(/\s*[-|]\s*抖音直播.*$/, "").replace(/的抖音直播间.*$/, "").replace(/的直播间.*$/, "");
    return t.trim();
  }

  function send(info) {
    try {
      window.__TAURI_INTERNALS__.invoke("douyin_stream", {
        info: Object.assign({ webRid: webRid(), anchor: anchorName(), pageUrl: location.href, userAgent: navigator.userAgent }, info),
      });
    } catch (e) {}
  }

  var playing = ""; // 頁面播放器目前在拉的那一路
  function report(url, source, quality, resolution) {
    if (!url || typeof url !== "string") return;
    url = url.replace(/\\u0026/g, "&").replace(/&amp;/g, "&");
    if (!STREAM.test(url) || !CDN.test(url)) return;
    if (!webRid()) return; // 只在直播間頁面回報
    var key = webRid() + "|" + url.split("?")[0];
    // 頁面切回之前播過的畫質時也要回報，SupaLive 才知道現在播的是哪一路
    var nowPlaying = source !== "api" && playing !== key;
    if (seen[key] && !nowPlaying) return;
    seen[key] = 1;
    if (source !== "api") playing = key;
    send({ url: url, source: source, quality: quality || null, resolution: resolution || null });
  }

  // 直播間資料 API 的回應裡有各畫質的網址，全部回報，讓使用者選要錄哪一種
  var ORDER = ["origin", "FULL_HD1", "uhd", "HD1", "hd", "SD1", "sd", "SD2", "ld"];
  function resolutionOf(q) {
    try {
      var p = typeof q.sdk_params === "string" ? JSON.parse(q.sdk_params) : q.sdk_params;
      return (p && p.resolution) || null;
    } catch (e) { return null; }
  }
  function scan(obj, depth) {
    if (!obj || typeof obj !== "object" || depth > 8) return;
    if (obj.flv_pull_url && typeof obj.flv_pull_url === "object") {
      for (var i = 0; i < ORDER.length; i++) {
        if (obj.flv_pull_url[ORDER[i]]) report(obj.flv_pull_url[ORDER[i]], "api", ORDER[i]);
      }
    }
    if (typeof obj.stream_data === "string" && obj.stream_data.indexOf("{") === 0) {
      try {
        var sd = JSON.parse(obj.stream_data).data || {};
        for (var j = 0; j < ORDER.length; j++) {
          var q = sd[ORDER[j]];
          if (q && q.main && q.main.flv) report(q.main.flv, "api", ORDER[j], resolutionOf(q.main));
        }
      } catch (e) {}
    }
    for (var k in obj) {
      if (Object.prototype.hasOwnProperty.call(obj, k) && obj[k] && typeof obj[k] === "object") scan(obj[k], depth + 1);
    }
  }

  var API = /webcast\/room\/web\/enter|webcast\/room\/info|room\/reflow/;

  var origFetch = window.fetch;
  if (origFetch) {
    window.fetch = function (input) {
      var url = "";
      try {
        url = typeof input === "string" ? input : (input && input.url) || "";
        report(url, "fetch");
      } catch (e) {}
      var p = origFetch.apply(this, arguments);
      if (API.test(url)) {
        p.then(function (r) { return r.clone().json(); }).then(function (j) { scan(j, 0); }).catch(function () {});
      }
      return p;
    };
  }

  var origOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    try {
      report(String(url), "xhr");
      if (API.test(String(url))) {
        this.addEventListener("load", function () {
          try { scan(JSON.parse(this.responseText), 0); } catch (e) {}
        });
      }
    } catch (e) {}
    return origOpen.apply(this, arguments);
  };

  // 頁面伺服器端渲染時直接寫在 HTML 裡的網址
  var FLV_IN_TEXT = /https?:(?:\\?\/){2}[^"'\s<>]+?\.flv[^"'\s<>]*/g;
  function scanDocument() {
    if (!webRid()) return;
    var scripts = document.querySelectorAll("script:not([src])");
    for (var i = 0; i < scripts.length; i++) {
      var text = scripts[i].textContent || "";
      if (text.indexOf(".flv") < 0) continue;
      var m = text.match(FLV_IN_TEXT) || [];
      for (var j = 0; j < m.length; j++) report(m[j].replace(/\\\//g, "/"), "html");
    }
  }
  var tries = 0;
  var timer = setInterval(function () {
    scanDocument();
    if (++tries > 15) clearInterval(timer);
  }, 2000);

  // 單頁應用切換直播間時重新開始
  var lastPath = location.pathname;
  setInterval(function () {
    if (location.pathname !== lastPath) {
      lastPath = location.pathname;
      tries = 0;
      clearInterval(timer);
      timer = setInterval(function () {
        scanDocument();
        if (++tries > 15) clearInterval(timer);
      }, 2000);
    }
  }, 1000);
})();
