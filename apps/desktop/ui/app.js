// SupaLive 桌面版介面。版面與互動沿用已確認的原型，資料改由 Rust 端提供。
(function () {
  "use strict";
  var T = window.__TAURI__;
  var invoke = T.core.invoke;
  var listen = T.event.listen;

  var SEC = 1000;
  var I = {
    plus: '<path d="M12 5v14M5 12h14"/>',
    set: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
    play: '<path d="M8 5l11 7-11 7z" fill="currentColor" stroke="none"/>',
    pause: '<path d="M8 5v14M16 5v14" stroke-width="3"/>',
    stop: '<rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" stroke="none"/>',
    rec: '<circle cx="12" cy="12" r="6" fill="currentColor" stroke="none"/>',
    vol: '<path d="M4 10v4h4l5 4V6L8 10z"/><path d="M17 9a4 4 0 010 6"/>',
    mute: '<path d="M4 10v4h4l5 4V6L8 10z"/><path d="M17 10l4 4M21 10l-4 4"/>',
    folder: '<path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/>',
    link: '<path d="M10 14a4 4 0 006 0l3-3a4 4 0 00-6-6l-1 1"/><path d="M14 10a4 4 0 00-6 0l-3 3a4 4 0 006 6l1-1"/>',
    user: '<circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0116 0"/>',
    x: '<path d="M6 6l12 12M18 6L6 18"/>',
    back: '<path d="M15 5l-7 7 7 7"/>',
    warn: '<path d="M12 3l10 18H2z"/><path d="M12 10v5M12 18v.5"/>',
    eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
    grid: '<rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/>',
    reload: '<path d="M20 12a8 8 0 11-2.3-5.7M20 4v5h-5"/>',
    trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
    chev: '<path d="M9 6l6 6-6 6"/>'
  };
  function ic(n) { return '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + I[n] + "</svg>"; }
  function esc(s) { return String(s == null ? "" : s).replace(/[&<>"]/g, function (c) { return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]; }); }
  function $(id) { return document.getElementById(id); }

  // ---------------------------------------------------------------- 語言
  // 介面文字以正體中文寫在程式裡，英文從 i18n.js 查，查不到就顯示中文。{n} 這類是要代入的值
  var EN = (window.SupaI18n && window.SupaI18n.EN) || {};
  var LANG = "zh-Hant";
  function t(zh, v) {
    var s = LANG === "en" && EN[zh] != null ? EN[zh] : zh;
    return v ? s.replace(/\{(\w+)\}/g, function (m, k) { return v[k] != null ? v[k] : m; }) : s;
  }
  // 跟隨系統：取系統偏好語言裡第一個中文或英文，都沒有時用正體中文
  function sysLang() {
    var ls = navigator.languages && navigator.languages.length ? navigator.languages : [navigator.language || ""];
    for (var i = 0; i < ls.length; i++) {
      var l = String(ls[i]).toLowerCase();
      if (l.indexOf("zh") === 0) return "zh-Hant";
      if (l.indexOf("en") === 0) return "en";
    }
    return "zh-Hant";
  }
  var sentLang = null;
  function applyLang() {
    var pref = S.settings && S.settings.lang;
    LANG = pref === "en" || pref === "zh-Hant" ? pref : sysLang();
    document.documentElement.lang = LANG;
    // index.html 裡寫死的無障礙標籤
    [["[data-act=win-min]", "最小化"], ["[data-act=win-max]", "最大化"], ["[data-act=win-close]", "關閉"], ["#side", "直播間"], ["#rz-side", "調整側欄寬度"], ["#rz-dy", "調整選取直播的寬度"]].forEach(function (a) {
      var el = document.querySelector(a[0]); if (el) el.setAttribute("aria-label", t(a[1]));
    });
    // 告訴後端，之後的錯誤和狀態訊息用同一種語言
    if (sentLang !== LANG) { sentLang = LANG; invoke("set_locale", { lang: LANG }).catch(function () {}); }
  }

  // ---------------------------------------------------------------- 狀態
  var S = {
    view: "grid", sel: null, recIdx: null, modal: null, addText: "", addErr: "",
    fold: {}, more: {}, // 首頁收起的直播間、展開全部錄製的直播間
    // cands：webRid -> 抓到的各種畫質；pick：webRid -> 使用者選的那一路；rid：目前側欄裡的直播間；goto：下次打開側欄要跳的網址
    dy: { open: false, url: "live.douyin.com", cands: {}, pick: {}, rid: null, goto: null },
    p: { muted: true, vol: 70, clip: null },
    settings: null, rooms: [], covers: {}, present: {}, platform: "", material: "none", privateApi: true,
    live: {} // roomId -> { bytes, samples: [[t, bytes]] }
  };

  function pad(n) { return String(n).padStart(2, "0"); }
  function dur(s) { s = Math.max(0, Math.floor(s)); var h = (s / 3600) | 0, m = ((s / 60) | 0) % 60; return (h ? h + ":" + pad(m) : m) + ":" + pad(s % 60); }
  function bytes(b) { if (!b) return "0 MB"; if (b < 1e9) return (b / 1e6).toFixed(0) + " MB"; return (b / 1e9).toFixed(2) + " GB"; }
  function base(p) { return String(p || "").split(/[\\/]/).pop(); }
  function when(t) { var d = new Date(t); return d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate()) + " " + pad(d.getHours()) + ":" + pad(d.getMinutes()); }

  // 錄製的檔案還在不在：後端每次讀狀態時檢查，使用者可能在 App 外面搬走或刪掉
  function presOf(r, i) { var a = S.present[r.id]; return (a && a[i]) || { dir: true, missing: [] }; }
  function existing(list, pr) { return pr.dir ? list.filter(function (f) { return pr.missing.indexOf(f) < 0; }) : []; }
  // 廢紙簍在 Windows 叫資源回收筒
  function bin() { return t(S.platform === "windows" ? "資源回收筒" : "廢紙簍"); }

  function lastRec(r) { return r.recordings.length ? r.recordings[r.recordings.length - 1] : null; }
  function status(r) { var x = lastRec(r); return x ? x.status : "idle"; }
  function title(r) { return r.name || (r.anchor ? r.anchor.replace(/^@/, "") : "") || t("未命名直播間"); }
  function room(id) { for (var i = 0; i < S.rooms.length; i++) if (S.rooms[i].id == id) return S.rooms[i]; }
  function elapsed(r) { var x = lastRec(r); if (!x) return 0; return Math.max(0, ((x.ended || Date.now()) - x.started) / SEC); }
  function recBytes(r) { var x = lastRec(r); if (!x) return 0; var l = S.live[r.id]; return Math.max(x.bytes || 0, (l && x.status === "rec" && l.bytes) || 0); }
  function segLen() { return S.settings.segMin * 60; }
  // 已完成的依最近一次錄製排，新的在前
  function byRecent(a, b) { var x = lastRec(a), y = lastRec(b); return ((y && (y.ended || y.started)) || 0) - ((x && (x.ended || x.started)) || 0); }
  // 首頁的直播間：錄製中的在前，其餘依最近一次活動排，新的在前
  function lastActive(r) { var x = lastRec(r); return x ? x.ended || x.started : r.addedAt || 0; }
  function byActive(a, b) {
    var ra = status(a) === "rec", rb = status(b) === "rec";
    return ra !== rb ? (ra ? -1 : 1) : lastActive(b) - lastActive(a);
  }
  function nRec() { return S.rooms.filter(function (r) { return status(r) === "rec"; }).length; }
  function kbps(r) {
    var l = S.live[r.id]; if (!l || l.samples.length < 2) return null;
    var a = l.samples[0], b = l.samples[l.samples.length - 1];
    if (b[0] - a[0] < 1500) return null;
    return ((b[1] - a[1]) * 8) / ((b[0] - a[0]) / SEC) / 1e3;
  }
  function expireText(url) {
    var m = /[?&]expire=(\d+)/.exec(url || ""); if (!m) return "";
    var s = +m[1] * SEC - Date.now();
    if (s <= 0) return t("連結已過期");
    return s > 48 * 3600 * SEC ? t("連結約 {n} 天後到期", { n: Math.round(s / 86400000) }) : t("連結約 {n} 小時後到期", { n: Math.max(1, Math.round(s / 3600000)) });
  }
  var LBL = { rec: "錄製中", idle: "待錄製", merging: "合併中", done: "已完成" };

  // 沒有畫面時的播放器底：深色，有封面時模糊鋪在後面
  function ph(r, cls, inner, c) {
    return '<div class="vid ' + (cls || "") + '">' + (c ? '<span class="snap" style="background-image:url(' + coverUrl(c) + ')"></span>' : "") + (inner || "") + '<span class="who">' + esc(title(r)) + "</span></div>";
  }

  // ---------------------------------------------------------------- 側欄
  function renderSide() {
    var groups = [["錄製中", ["rec"]], ["直播間", ["idle"]], ["已完成", ["merging", "done"]]];
    var h = "<h4>" + t("總覽") + "</h4><button class=\"si" + (S.view === "grid" ? " on" : "") + '" data-act="home">' + ic("grid") + '<span class="nm">' + t("所有錄製") + "</span><small>" + nRec() + "</small></button>";
    groups.forEach(function (g) {
      var rs = S.rooms.filter(function (r) { return g[1].indexOf(status(r)) >= 0; });
      if (g[1][0] === "merging") rs.sort(byRecent);
      if (!rs.length) return;
      h += "<h4>" + t(g[0]) + "</h4>";
      rs.forEach(function (r) {
        var st = status(r);
        var dot = st === "idle" ? "" : st;
        h += '<button class="si' + (S.view === "player" && S.sel == r.id ? " on" : "") + '" data-act="open" data-id="' + r.id + '"><span class="dot ' + dot + '"></span><span class="nm">' + esc(title(r)) + "</span>" + (st === "rec" ? '<small data-b="dur:' + r.id + '"></small>' : "") + "</button>";
      });
    });
    h += '<div class="grow"></div><h4>' + t("工具") + "</h4>";
    h += '<button class="si" data-act="add">' + ic("plus") + '<span class="nm">' + t("新增直播間") + "</span></button>";
    h += '<button class="si' + (S.dy.open ? " on" : "") + '" data-act="dytoggle">' + ic("user") + '<span class="nm">' + t("選取直播") + "</span></button>";
    h += '<button class="si" data-act="settings">' + ic("set") + '<span class="nm">' + t("設定") + "</span></button>";
    $("side").innerHTML = h;
  }

  // ---------------------------------------------------------------- 主區：總覽
  function renderGrid() {
    var rooms = S.rooms.slice().sort(byActive), n = nRec();
    var h = '<div class="mh"><div><h1>' + t("直播間") + '</h1><p class="sub">' + (rooms.length ? t("{n} 個直播間", { n: rooms.length }) + (n ? t("，{n} 個正在錄製，存到 ", { n: n }) + '<span style="font-family:var(--f-mono)" class="selectable">' + esc(S.settings.root) + "</span>" : "") : t("還沒有直播間")) + "</p></div>" +
      '<button class="btn primary" data-act="add">' + ic("plus") + t("新增直播間") + "</button></div>";
    if (n > 2 && S.settings.warnMulti) h += '<div class="banner" role="status">' + ic("warn") + "<p><b>" + t("同時錄製 {n} 個直播間。", { n: n }) + "</b>" + t("超過 2 個可能觸發抖音風控，拉流會被暫時封鎖。如果開始斷線，先停掉幾個。") + "</p></div>";
    if (!rooms.length) return h + '<div class="empty">' + t("按「新增直播間」貼上直播間網址，或打開「選取直播」進入直播間。") + "</div>";
    return h + '<div class="rooms">' + rooms.map(roomBlock).join("") + "</div>";
  }

  // 第一級：直播間。底下列出它的每次錄製，新的在前，預設只列最近 3 次
  function roomBlock(r) {
    var st = status(r), n = r.recordings.length, fold = !!S.fold[r.id], exp = expireText(r.url);
    var sub = [r.anchor && r.anchor.replace(/^@/, "") !== title(r) ? r.anchor : "", t(r.quality), n ? t("{n} 次錄製", { n: n }) : t("還沒有錄製"), st === "rec" ? "" : exp];
    var h = '<div class="row rhead">' +
      (n ? '<button class="iconbtn fold' + (fold ? "" : " on") + '" data-act="fold" data-id="' + r.id + '" aria-expanded="' + !fold + '" aria-label="' + t(fold ? "展開錄製紀錄" : "收起錄製紀錄") + '">' + ic("chev") + "</button>" : '<span class="fold"></span>') +
      '<div class="t"><b>' + esc(title(r)) + "</b><small>" + esc(sub.filter(Boolean).join(" · ")) + "</small></div>" +
      (st === "rec" || st === "merging" ? '<span class="pill ' + st + '">' + t(LBL[st]) + "</span>" : "") +
      '<div class="acts">' +
      // 連結過期時可以回到直播間重新抓：要有直播間 ID（從選取直播加入的才有）
      (st !== "rec" && exp === t("連結已過期") && r.webRid ? '<button class="btn sm" data-act="relink" data-id="' + r.id + '">' + ic("reload") + t("重新抓取連結") + "</button>" : "") +
      (st === "rec"
        ? '<button class="btn sm red" data-act="stop" data-id="' + r.id + '">' + ic("stop") + t("停止") + "</button>"
        : '<button class="btn sm red" data-act="start" data-id="' + r.id + '"' + (exp === t("連結已過期") || st === "merging" ? " disabled" : "") + ">" + ic("rec") + t(n ? "再錄一次" : "開始錄製") + "</button>") +
      '<button class="iconbtn x" data-act="remove" data-id="' + r.id + '" aria-label="' + t("刪除直播間") + '">' + ic("trash") + "</button></div></div>";
    if (n && !fold) {
      var idx = r.recordings.map(function (x, i) { return i; }).reverse();
      var cut = !S.more[r.id] && idx.length > 3;
      h += '<div class="sessions">' + (cut ? idx.slice(0, 3) : idx).map(function (i) { return sessRow(r, i); }).join("") +
        (cut ? '<button class="more" data-act="more" data-id="' + r.id + '">' + t("顯示全部 {n} 次錄製", { n: n }) + "</button>" : "") + "</div>";
    }
    return '<section class="room glass">' + h + "</section>";
  }

  // 第二級：一次錄製。正在錄的那一次顯示即時預覽
  function sessRow(r, i) {
    var x = r.recordings[i];
    if (x.status === "rec") {
      var codec = x.codec ? x.codec + (x.height ? " · " + x.width + "x" + x.height : "") : t("連線中");
      return '<div class="sess live">' +
        '<button class="vid live" data-act="open" data-id="' + r.id + '" aria-label="' + esc(t("觀看 {name}", { name: title(r) })) + '">' +
        snapEl(r.id) + '<span data-pv="' + r.id + '"></span><span class="wait">' + t("正在連接畫面…") + "</span>" +
        '<span class="chips"><span class="chip rec">● REC</span><span class="chip">' + esc(codec) + "</span></span>" +
        '<span class="watch btn sm">' + ic("eye") + t("邊錄邊看") + "</span></button>" +
        '<div class="sinfo"><div class="cmeta"><div class="t"><b>' + t("{time} 開始", { time: when(x.started) }) + "</b><small>" + esc(x.note ? t("注意：{note}", { note: x.note }) : "") + "</small></div></div>" +
        '<dl class="stats"><div><dt>' + t("時長") + '</dt><dd data-b="dur:' + r.id + '"></dd></div><div><dt>' + t("大小") + '</dt><dd data-b="size:' + r.id + '"></dd></div><div><dt>' + t("分段") + '</dt><dd data-b="seg:' + r.id + '"></dd></div><div><dt>' + t("碼率") + '</dt><dd data-b="kbps:' + r.id + '"></dd></div></dl>' +
        '<div class="prog"><i data-w="segp:' + r.id + '"></i></div>' +
        '<div class="hint" data-b="seghint:' + r.id + '"></div></div></div>';
    }
    // 只算還找得到的檔案；整個資料夾不見時只剩刪除（從清單拿掉）
    var pr = presOf(r, i), merged = existing(x.merged, pr).length, nclips = existing(x.clips, pr).length, at = ' data-id="' + r.id + '" data-i="' + i + '"';
    var info = [dur(Math.max(0, ((x.ended || x.started) - x.started) / SEC)), bytes(x.bytes), t("{n} 個分段", { n: nclips }) + (merged && !nclips && !pr.missing.length ? t("（已刪除）") : "")];
    if (!pr.dir) info = [t("找不到錄製資料夾，可能已被移動或刪除")];
    else if (pr.missing.length) info.push(t("{n} 個檔案找不到", { n: pr.missing.length }));
    if (x.note && !nclips && pr.dir) info.push(x.note);
    var del = '<button class="iconbtn x" data-act="rmrec"' + at + ' aria-label="' + t("刪除這次錄製") + '">' + ic("trash") + "</button>";
    var right = x.status === "merging"
      ? '<span class="pill merging">' + t("合併中") + "</span>"
      : !pr.dir
      ? '<span class="pill missing">' + t("找不到檔案") + '</span><div class="acts">' + del + "</div>"
      : '<span class="pill done">' + t(merged ? "已合併" : "未合併") + '</span><div class="acts">' +
        (nclips || merged ? '<button class="btn sm" data-act="openrec"' + at + ">" + ic("play") + t("播放") + "</button>" : "") +
        (!merged && nclips ? '<button class="btn sm" data-act="merge"' + at + ">" + t("合併") + "</button>" : "") +
        '<button class="btn sm ghost" data-act="reveal" data-path="' + esc(x.dir) + '">' + ic("folder") + t("打開資料夾") + "</button>" + del + "</div>";
    var c = coverOf(r, i);
    return '<div class="row sess"><span class="thumb"' + (c ? ' style="background-image:url(' + coverUrl(c) + ')"' : "") + '></span><div class="t"><b>' + when(x.started) + "</b><small>" + esc(info.join(" · ")) + "</small></div>" + right + "</div>";
  }

  // ---------------------------------------------------------------- 主區：播放器
  var player = null; // 目前的 LivePlayer

  // 播放器頁看的是哪一次錄製：從首頁的某次錄製點進來就是那一次，否則是最近一次
  function recIndex(r) { var n = r.recordings.length; return S.recIdx != null && S.recIdx < n ? S.recIdx : n - 1; }

  function renderPlayer() {
    var r = room(S.sel); if (!r) { S.view = "grid"; return renderGrid(); }
    var rst = status(r), x = r.recordings[recIndex(r)] || null, st = x ? x.status : "idle";
    var head = '<div class="pbar"><button class="iconbtn" data-act="home" aria-label="' + t("返回") + '">' + ic("back") + "</button><h1>" + esc(title(r)) + "</h1>" + (x ? '<span class="sub">' + when(x.started) + "</span>" : "") + '<span class="pill ' + (st === "idle" ? "offline" : st) + '">' + t(LBL[st]) + '</span><span class="grow"></span>' +
      (rst === "rec" ? '<button class="btn red" data-act="stop" data-id="' + r.id + '">' + ic("stop") + t("停止錄製") + "</button>" : "") +
      (rst === "idle" || rst === "done" ? '<button class="btn red" data-act="start" data-id="' + r.id + '">' + ic("rec") + t("開始錄製") + "</button>" : "") + "</div>";
    var chips = '<span class="chips">' + (st === "rec" ? '<span class="chip rec">● REC</span><span class="chip live" data-b="lat">LIVE</span>' : "") + (x && x.codec ? '<span class="chip">' + esc(x.codec + (x.height ? " · " + x.width + "x" + x.height : "")) + "</span>" : "") + "</span>";
    // 畫面框依影片比例，沒有寬高時當作直式
    var ar = x && x.width && x.height ? x.width + "/" + x.height : "9/16";
    var stage = '<div class="stage" style="--ar:' + ar + '">';
    if (st === "rec") {
      stage += '<video id="pv" playsinline muted></video>' + snapEl(r.id) + chips + '<div class="msg" id="pmsg"><p>' + t("正在連接錄製中的畫面…") + "</p></div>";
      stage += '<div class="ctrl glass">' +
        '<button class="iconbtn" data-act="pp" id="ppbtn" aria-label="' + t("暫停") + '">' + ic("pause") + "</button>" +
        '<span class="tm" data-b="ptime"></span>' +
        '<div class="tl" id="tl" role="slider" tabindex="0" aria-label="' + t("回看時間軸") + '"><span class="track"></span><span class="fill" data-w="pfill"></span><span class="knob" data-l="pknob"></span></div>' +
        '<button class="golive" data-act="golive" data-b="golive"></button>' +
        '<button class="iconbtn" data-act="mute" id="mutebtn" aria-label="' + t(S.p.muted ? "取消靜音" : "靜音") + '">' + ic(S.p.muted ? "mute" : "vol") + "</button>" +
        '<input class="vol" id="vol" type="range" min="0" max="100" value="' + S.p.vol + '" aria-label="' + t("音量") + '">' +
        "</div>";
    } else if (x && existing(x.merged.concat(x.clips), presOf(r, recIndex(r))).length) {
      // 只播還找得到的檔案
      var files = existing(x.merged.concat(x.clips), presOf(r, recIndex(r)));
      if (!S.p.clip || files.indexOf(S.p.clip) < 0) S.p.clip = files[0];
      // 系統播放器的控制項會用到畫面四角，這裡不疊標籤，編碼和解析度寫在右邊的串流資訊裡
      stage += '<video id="pv" controls playsinline src="' + esc(T.core.convertFileSrc(S.p.clip)) + '"></video><div class="msg" id="pmsg" hidden></div>';
    } else {
      var gone = x && (x.merged.length || x.clips.length || !presOf(r, recIndex(r)).dir);
      var why = st === "merging" ? "正在合併分段…" : !gone ? "還沒有錄製。按「開始錄製」才會存檔。" : presOf(r, recIndex(r)).dir ? "找不到這次錄製的檔案，可能已被移動或刪除。" : "找不到這次錄製的資料夾，可能已被移動或刪除。";
      stage += ph(r, "", '<span class="center">' + t(why) + "</span>", x && coverOf(r, recIndex(r)));
    }
    stage += "</div>";

    var kv = "<dt>" + t("主播") + '</dt><dd class="selectable">' + esc(r.anchor || "—") + "</dd>" +
      "<dt>" + t("來源") + "</dt><dd>" + t("抖音 CDN · FLV 拉流") + "</dd>" +
      (x && x.codec ? "<dt>" + t("編碼") + "</dt><dd>" + esc(x.codec) + (x.codec === "HEVC" ? t("（FLV codec 12 → hvc1）") : "") + "</dd>" : "") +
      (x && x.height ? "<dt>" + t("解析度") + "</dt><dd>" + x.width + "x" + x.height + "</dd>" : "") +
      "<dt>" + t("畫質") + "</dt><dd>" + esc(t(r.quality || "預設")) + (expireText(r.url) ? " · " + expireText(r.url) : "") + "</dd>" +
      "<dt>" + t("存檔") + "</dt><dd>" + t("fMP4，只重新封裝，不重新編碼") + "</dd>" +
      (st === "rec" ? "<dt>" + t("已錄") + '</dt><dd data-b="dur:' + r.id + '"></dd><dt>' + t("大小") + '</dt><dd data-b="size:' + r.id + '"></dd>' +
        "<dt>" + t("碼率") + '</dt><dd data-b="kbps:' + r.id + '"></dd><dt>' + t("幀率") + '</dt><dd data-b="fps:' + r.id + '"></dd>' : "");
    var info = '<div class="pinfo"><section class="box glass"><h3>' + t("串流") + '</h3><dl class="kv">' + kv + "</dl>" +
      (st === "rec" ? '<p class="note">' + t("播放的是正在寫入的錄影，可以往回拖最近約 3 分鐘，按「直播」回到最新。更早的部分在已完成的分段裡。") + "</p>" : "") + "</section>";
    if (x) {
      var px = presOf(r, recIndex(r));
      info += '<section class="box glass"><h3>' + t("檔案") + '</h3><pre class="tree selectable">' + tree(x, coverOf(r, recIndex(r)), px) + "</pre>";
      if (st !== "rec" && (x.merged.length || x.clips.length) && px.dir) {
        // 找不到的檔案留在清單上但不能點，看得出少了哪些
        info += '<ul class="cliplist">' + x.merged.concat(x.clips).map(function (f) {
          var lost = px.missing.indexOf(f) >= 0;
          return '<li><button class="' + (f === S.p.clip ? "on" : "") + (lost ? " gone" : "") + '"' + (lost ? " disabled" : ' data-act="clip"') + ' data-path="' + esc(f) + '"><span>' + esc(base(f)) + "</span><span>" + (lost ? t("找不到") : t(x.merged.indexOf(f) >= 0 ? "合併檔" : "分段")) + "</span></button></li>";
        }).join("") + "</ul>";
      }
      if (px.dir) info += '<div><button class="btn sm ghost" data-act="reveal" data-path="' + esc(x.dir) + '">' + ic("folder") + t("打開資料夾") + "</button></div>";
      info += "</section>";
    }
    info += "</div>";
    return '<div class="pv">' + head + '<div class="pbody">' + stage + info + "</div></div>";
  }

  function tree(x, c, px) {
    var root = String(x.dir).replace(/[\\/][^\\/]+$/, "");
    if (px && !px.dir) return '<span class="m">' + esc(root) + "/</span>\n└─ " + esc(base(x.dir)) + '/  <span class="r">' + t("找不到資料夾") + "</span>\n";
    var out = '<span class="m">' + esc(root) + "/</span>\n└─ " + esc(base(x.dir)) + "/\n   ├─ clips/\n";
    var clips = x.clips.slice();
    if (x.status === "rec") clips.push(null);
    if (!clips.length) out += "   │  " + '<span class="m">' + t(x.merged.length ? "（合併後已刪除）" : "（沒有分段）") + "</span>\n";
    var show = clips;
    if (show.length > 4) { out += '   │  <span class="m">' + t("… 前 {n} 段", { n: show.length - 3 }) + "</span>\n"; show = show.slice(-3); }
    show.forEach(function (c, i) {
      var last = i === show.length - 1;
      var lost = c && px && px.missing.indexOf(c) >= 0;
      out += "   │  " + (last ? "└" : "├") + "─ " + (c ? esc(base(c)) + (lost ? '  <span class="r">' + t("找不到") + "</span>" : "") : '<span class="r">' + t("● 寫入中") + "</span>") + "\n";
    });
    if (c) out += "   ├─ cover.jpg\n";
    out += "   └─ merged/\n";
    if (x.merged.length) x.merged.forEach(function (m) { out += px && px.missing.indexOf(m) >= 0 ? "      └─ " + esc(base(m)) + '  <span class="r">' + t("找不到") + "</span>\n" : '      └─ <span class="g">' + esc(base(m)) + "</span>\n"; });
    else if (x.status === "merging") out += '      └─ <span class="m">' + t("合併中…") + "</span>\n";
    else out += '      <span class="m">' + t(S.settings.autoMerge ? "錄製結束後自動產生" : "自動合併已關閉") + "</span>\n";
    return out;
  }

  // ---------------------------------------------------------------- 邊錄邊播（MSE）
  // opts.tail：開頭只要最近幾秒的回看（卡片預覽用）；opts.keep：緩衝保留幾秒；opts.quiet：不顯示訊息
  function LivePlayer(video, roomId, opts) {
    this.video = video; this.roomId = roomId; this.queue = []; this.sb = null; this.alive = true; this.follow = true;
    this.opts = opts || {}; this.keep = this.opts.keep || 180; this.wid = 0;
    var self = this;
    this.ms = new MediaSource();
    video.src = URL.createObjectURL(this.ms);
    this.ms.addEventListener("sourceopen", function () { self.subscribe(); }, { once: true });
    // 開始出畫面後才蓋住最後一幅畫面，避免先閃一下黑底
    video.addEventListener("playing", function () {
      video.classList.add("on");
      // 一出畫面就先留一張，直式畫面兩側馬上有模糊底色
      setTimeout(function () { self.snapshot(); showSnap(roomId); }, 300);
    });
  }
  LivePlayer.prototype.msg = function (html) {
    if (this.opts.quiet) return;
    var m = $("pmsg"); if (!m) return;
    m.hidden = !html; if (html) m.innerHTML = html;
  };
  LivePlayer.prototype.subscribe = function () {
    var self = this;
    var ch = new T.core.Channel();
    ch.onmessage = function (data) { if (self.alive) self.onPacket(data); };
    this.channel = ch;
    invoke("watch", { id: this.roomId, tail: this.opts.tail || null, channel: ch })
      .then(function (wid) { self.wid = wid; if (!self.alive) self.unwatch(); })
      .catch(function (e) { self.msg("<p>" + esc(e) + "</p>"); });
  };
  // Rust 端不知道介面丟掉了 channel，不用時要明確停掉轉發
  LivePlayer.prototype.unwatch = function () {
    if (this.wid) invoke("unwatch", { watch: this.wid }).catch(function () {});
    this.wid = 0;
  };
  // 留下目前畫面的小縮圖，模糊後當作暫停或重新連接時的底圖
  LivePlayer.prototype.snapshot = function () {
    var v = this.video;
    if (!v.videoWidth || v.readyState < 2) return;
    try {
      var c = document.createElement("canvas");
      c.width = 192; c.height = Math.max(1, Math.round((192 * v.videoHeight) / v.videoWidth));
      c.getContext("2d").drawImage(v, 0, 0, c.width, c.height);
      snaps[this.roomId] = c.toDataURL("image/jpeg", 0.7);
    } catch (e) {}
  };
  LivePlayer.prototype.onPacket = function (data) {
    var u8 = data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(data);
    var kind = u8[0];
    if (kind === 0) {
      var len = (u8[1] << 8) | u8[2];
      var codecs = new TextDecoder().decode(u8.subarray(3, 3 + len));
      var body = u8.slice(3 + len);
      if (!this.sb) {
        var mime = 'video/mp4; codecs="' + codecs + '"';
        if (!window.MediaSource || !MediaSource.isTypeSupported(mime)) {
          var tip = /^hvc1|^hev1/.test(codecs)
            ? t("錄製不受影響。Windows 需要安裝「HEVC 視訊延伸模組」並有支援 HEVC 的顯示卡；也可以錄完用其他播放器打開。")
            : t("錄製不受影響，錄完可以用其他播放器打開。");
          this.msg("<p>" + t("這台電腦的系統 WebView 不支援播放這個編碼（{codec}）。", { codec: esc(codecs) }) + "<small>" + tip + "</small></p>");
          this.alive = false; this.unwatch();
          return;
        }
        try { this.sb = this.ms.addSourceBuffer(mime); } catch (e) { this.msg("<p>" + t("無法建立播放器：{err}", { err: esc(e.message) }) + "</p>"); this.alive = false; this.unwatch(); return; }
        var self = this;
        this.sb.addEventListener("updateend", function () { self.afterAppend(); });
        this.queue.push({ data: body });
      } else {
        // 新分段的時間從 0 開始，接在目前已緩衝的結尾後面
        this.queue.push({ data: body, newClip: true });
      }
    } else if (kind === 1) {
      this.queue.push({ data: u8.slice(1) });
    } else if (kind === 2) {
      this.msg("<p>" + t("錄製已結束") + "</p>");
      this.alive = false; this.wid = 0;
      return;
    }
    this.pump();
  };
  LivePlayer.prototype.pump = function () {
    if (!this.sb || this.sb.updating || !this.queue.length) return;
    var item = this.queue.shift();
    try {
      if (item.newClip && this.sb.buffered.length) this.sb.timestampOffset = this.sb.buffered.end(this.sb.buffered.length - 1);
      this.sb.appendBuffer(item.data);
    } catch (e) {
      // 超過瀏覽器的緩衝上限：丟掉較舊的部分再試
      if (e.name === "QuotaExceededError") { this.queue.unshift(item); this.trim(60); }
    }
  };
  LivePlayer.prototype.trim = function (keep) {
    var b = this.sb.buffered; if (!b.length || this.sb.updating) return;
    var start = b.start(0), cut = this.video.currentTime - keep;
    if (cut - start > 10) { try { this.sb.remove(start, cut); } catch (e) {} }
  };
  LivePlayer.prototype.afterAppend = function () {
    var v = this.video, b = this.sb.buffered;
    if (b.length) {
      var end = b.end(b.length - 1);
      // 片段長度跟著關鍵幀間隔走，可能好幾秒才來一段；落後的容許量要跟著放寬，否則每來一段就跳一次
      var step = end - (this.lastEnd || end);
      if (step > 0 && step < 15) this.fragDur = Math.max(step, (this.fragDur || 0) * 0.8);
      this.lastEnd = end;
      if (this.follow) {
        if (v.currentTime < b.start(0) || end - v.currentTime > this.lag() + 3) v.currentTime = Math.max(b.start(0), this.liveTime());
        if (v.paused && !this.userPaused) v.play().catch(function () {});
      }
      this.msg(null);
      if (b.start(0) < v.currentTime - this.keep - 20) this.trim(this.keep);
    }
    this.pump();
  };
  // 跟播時停在離最新畫面多遠：至少比一段片段長一點，下一段來之前不會播到底卡住
  LivePlayer.prototype.lag = function () { return Math.max(1.2, (this.fragDur || 0) + 0.8); };
  LivePlayer.prototype.liveTime = function () { var rg = this.range(); return rg ? Math.max(rg[0], rg[1] - this.lag()) : 0; };
  LivePlayer.prototype.range = function () {
    var b = this.sb && this.sb.buffered;
    if (!b || !b.length) return null;
    return [b.start(0), b.end(b.length - 1)];
  };
  LivePlayer.prototype.destroy = function () {
    this.snapshot();
    saveCover(this.roomId);
    this.alive = false;
    this.unwatch();
    try { this.video.removeAttribute("src"); this.video.load(); } catch (e) {}
  };

  // ---------------------------------------------------------------- 總覽卡片的即時預覽
  // 每個錄製中的直播間一個靜音的小播放器，只在總覽頁、視窗看得到時播放。
  // 離開時停掉省資源，留下最後一幅畫面固定模糊顯示，回來接上直播後再淡入即時畫面。
  var previews = {}; // roomId -> LivePlayer
  var snaps = {}; // roomId -> 最後一幅畫面（小張 JPEG 的 data URL）

  // 記憶體裡沒有最後一幅畫面時（例如剛打開 App），改用錄製資料夾裡的封面
  function snapEl(id) {
    var r = room(id), c = r && coverOf(r, r.recordings.length - 1);
    var url = snaps[id] || (c ? coverUrl(c) : "");
    return '<span class="snap" data-snap="' + id + '"' + (url ? ' style="background-image:url(' + url + ')"' : "") + "></span>";
  }

  // ---------------------------------------------------------------- 封面（錄製資料夾裡的 cover.jpg）
  // 只在畫面停止播放時寫：切頁面、切到別的 App、錄製結束。同一個直播間 30 秒內最多寫一次，畫面沒變就不寫
  var coverAt = {}, coverSent = {}; // roomId -> 上次寫入的時間、內容
  function coverOf(r, i) { var a = S.covers[r.id]; return (a && a[i]) || null; }
  // 檔案更新後網址跟著變，才不會吃到舊的快取
  function coverUrl(c) { return esc(T.core.convertFileSrc(c.path) + "?v=" + c.mtime); }
  function saveCover(id) {
    var d = snaps[id], now = Date.now();
    if (!d || coverSent[id] === d || now - (coverAt[id] || 0) < 30 * SEC) return;
    coverAt[id] = now; coverSent[id] = d;
    var b = atob(d.slice(d.indexOf(",") + 1)), data = new Array(b.length);
    for (var i = 0; i < b.length; i++) data[i] = b.charCodeAt(i);
    invoke("save_cover", { id: id, data: data }).catch(function () {});
  }
  function showSnap(id) {
    if (!snaps[id]) return;
    document.querySelectorAll('[data-snap="' + id + '"]').forEach(function (el) { el.style.backgroundImage = "url(" + snaps[id] + ")"; });
  }
  function syncPreviews() {
    var on = S.view === "grid" && !document.hidden;
    var slots = {};
    document.querySelectorAll("span[data-pv]").forEach(function (el) { slots[el.dataset.pv] = el; });
    Object.keys(previews).forEach(function (id) {
      var p = previews[id];
      // 總覽重繪後卡片還在（有新的空位），或者影片本來就還掛在畫面上，就繼續播
      if (on && (slots[id] || p.video.isConnected)) return;
      delete previews[id];
      p.destroy();
      if (p.video.isConnected) { var s = document.createElement("span"); s.dataset.pv = id; p.video.replaceWith(s); }
      showSnap(id);
    });
    if (!on) return;
    Object.keys(slots).forEach(function (id) {
      var p = previews[id];
      if (!p) {
        var v = document.createElement("video");
        v.muted = true; v.playsInline = true;
        p = previews[id] = new LivePlayer(v, +id, { tail: 3, keep: 10, quiet: true });
      }
      // 同一個工作裡搬移影片元素不會讓它暫停，重繪時畫面不中斷
      slots[id].replaceWith(p.video);
    });
  }
  document.addEventListener("visibilitychange", syncPreviews);
  // 播放器頁切到別的 App 時不停播，但也留一張封面
  document.addEventListener("visibilitychange", function () {
    if (document.hidden && player) { player.snapshot(); saveCover(player.roomId); }
  });
  // 播放中也定期更新縮圖：直式畫面兩側的模糊底色跟著變，萬一停掉時抓不到畫面也有最近的一張
  setInterval(function () {
    Object.keys(previews).forEach(function (id) { previews[id].snapshot(); showSnap(id); });
  }, 10 * SEC);

  function setupPlayer() {
    if (player) { player.destroy(); player = null; }
    var r = room(S.sel);
    var v = $("pv");
    if (!r || !v) return;
    v.volume = S.p.vol / 100;
    if ((r.recordings[recIndex(r)] || {}).status === "rec") {
      v.muted = S.p.muted;
      player = new LivePlayer(v, r.id);
      var tl = $("tl");
      var seek = function (e) {
        var rg = player && player.range(); if (!rg) return;
        var b = tl.getBoundingClientRect(), f = Math.min(1, Math.max(0, (e.clientX - b.left) / b.width));
        var at = rg[0] + f * (rg[1] - rg[0]);
        player.follow = rg[1] - at < player.lag() + 2;
        v.currentTime = player.follow ? player.liveTime() : at;
        tick();
      };
      tl.onpointerdown = function (e) { tl.setPointerCapture(e.pointerId); seek(e); tl.onpointermove = seek; };
      tl.onpointerup = function () { tl.onpointermove = null; };
      tl.onkeydown = function (e) {
        var rg = player && player.range(); if (!rg) return;
        if (e.key === "ArrowLeft") { player.follow = false; v.currentTime = Math.max(rg[0], v.currentTime - 10); }
        if (e.key === "ArrowRight") { var to = v.currentTime + 10; player.follow = to >= rg[1] - player.lag() - 2; v.currentTime = player.follow ? player.liveTime() : to; }
        tick();
      };
      v.addEventListener("pause", function () { syncPP(); });
      v.addEventListener("play", function () { syncPP(); });
    }
    else {
      v.addEventListener("error", function () {
        // 檔案可能剛在 App 外面被刪掉：重新檢查，真的不見了就會重繪成「找不到」
        recheckFiles();
        var m = $("pmsg"); if (!m) return;
        var hevc = /265|hevc/i.test((r.recordings[recIndex(r)] || {}).codec || "");
        m.innerHTML = "<p>" + t("系統 WebView 無法播放這個檔案。") + "<small>" +
          (hevc ? t("Windows 需要安裝「HEVC 視訊延伸模組」。") : "") + t("檔案本身沒問題，可以按「打開資料夾」用其他播放器打開。") + "</small></p>";
        m.hidden = false;
      });
    }
    var vol = $("vol");
    if (vol) vol.oninput = function () { S.p.vol = +vol.value; v.volume = S.p.vol / 100; if (S.p.muted && S.p.vol > 0) { S.p.muted = false; v.muted = false; syncMute(); } };
  }
  function syncPP() { var v = $("pv"), b = $("ppbtn"); if (v && b) { b.innerHTML = ic(v.paused ? "play" : "pause"); b.setAttribute("aria-label", t(v.paused ? "播放" : "暫停")); } }
  function syncMute() { var b = $("mutebtn"); if (b) { b.innerHTML = ic(S.p.muted ? "mute" : "vol"); b.setAttribute("aria-label", t(S.p.muted ? "取消靜音" : "靜音")); } }

  function renderMain() {
    var pr = room(S.sel);
    var keepPlayer = player && S.view === "player" && pr && status(pr) === "rec" && recIndex(pr) === pr.recordings.length - 1 && player.roomId == S.sel && $("pv");
    if (keepPlayer) { tick(); return; } // 錄製中的播放器不重繪，避免畫面中斷
    // 換頁前先留下畫面，新頁面一畫出來就能用上最新的一幅
    if (player) player.snapshot();
    if (S.view !== "grid") Object.keys(previews).forEach(function (id) { previews[id].snapshot(); });
    $("main").innerHTML = S.view === "player" ? renderPlayer() : renderGrid();
    if (S.view === "player") setupPlayer();
    else if (player) { player.destroy(); player = null; }
    syncPreviews();
    tick();
  }

  // ---------------------------------------------------------------- 可拖曳的欄寬
  // 寬度只記在這台電腦的瀏覽器儲存裡，讀不到就用預設值
  var PANES = {
    side: { v: "--side-w", def: 224, min: 180, max: 360, dir: 1 },
    dy: { v: "--dy-w", def: 340, min: 300, max: 720, dir: -1 }
  };
  function setPane(k, w, save) {
    var p = PANES[k];
    w = Math.round(Math.min(p.max, Math.max(p.min, w)));
    document.documentElement.style.setProperty(p.v, w + "px");
    p.w = w;
    if (save) { try { localStorage.setItem("pane-" + k, String(w)); } catch (e) {} }
  }
  function setupResizers() {
    Object.keys(PANES).forEach(function (k) {
      var p = PANES[k], saved = null;
      try { saved = parseInt(localStorage.getItem("pane-" + k), 10); } catch (e) {}
      setPane(k, saved > 0 ? saved : p.def, false);
      var h = $("rz-" + k);
      h.addEventListener("pointerdown", function (e) {
        e.preventDefault();
        h.setPointerCapture(e.pointerId);
        h.classList.add("on");
        document.documentElement.classList.add("resizing");
        var x0 = e.clientX, w0 = p.w;
        h.onpointermove = function (ev) { setPane(k, w0 + (ev.clientX - x0) * p.dir, false); };
        h.onpointerup = h.onpointercancel = function () {
          h.onpointermove = h.onpointerup = h.onpointercancel = null;
          h.classList.remove("on");
          document.documentElement.classList.remove("resizing");
          setPane(k, p.w, true);
        };
      });
      h.addEventListener("dblclick", function () { setPane(k, p.def, true); });
      h.addEventListener("keydown", function (e) {
        var d = e.key === "ArrowLeft" ? -16 : e.key === "ArrowRight" ? 16 : 0;
        if (d) { e.preventDefault(); setPane(k, p.w + d * p.dir, true); }
      });
    });
  }

  // ---------------------------------------------------------------- 抖音側欄
  function renderDy() {
    var el = $("dy");
    el.hidden = !S.dy.open;
    $("rz-dy").hidden = !S.dy.open;
    if (!S.dy.open) {
      // 停止追蹤版面，免得關掉後還有尺寸變化把側欄又打開
      if (dyRO) { dyRO.disconnect(); dyRO = null; }
      invoke("douyin_close").catch(function () {});
      return;
    }
    var c = curCands();
    var h = '<div class="addr"><button class="iconbtn" data-act="dyback" aria-label="' + t("上一頁") + '">' + ic("back") + '</button><span class="url">' + esc(S.dy.url.replace(/^https?:\/\//, "")) + '</span><button class="iconbtn" data-act="dyreload" aria-label="' + t("重新整理") + '">' + ic("reload") + '</button><button class="iconbtn" data-act="dytoggle" aria-label="' + t("關閉選取直播") + '">' + ic("x") + "</button></div>" +
      '<div class="web" id="dyweb"><div class="ph">' + t("正在打開抖音…") + '</div></div><div class="sniffbar">';
    if (!c || !c.streams.length) {
      h += "<span>" + (/live\.douyin\.com\/\d+/.test(S.dy.url) ? '<span class="spin"></span>' + t("正在偵測拉流網址…") : t("登入後進入任何直播間，SupaLive 會自動抓到它的拉流網址。")) + "</span>";
    } else {
      var exist = findRoomFor(c), sel = pickOf(c);
      h += '<span class="ok">' + t("已抓到「{name}」的 {n} 種畫質", { name: esc(c.anchor || t("未命名直播間")), n: c.streams.length }) + "</span>" +
        '<div class="qlist" role="radiogroup" aria-label="' + t("要錄的畫質") + '">' + c.streams.map(function (q) {
          var on = q === sel;
          return '<button class="q' + (on ? " on" : "") + '" role="radio" aria-checked="' + on + '" data-act="dypick" data-k="' + esc(skey(q.url)) + '"><b>' + esc(t(q.label)) + "</b><span>" + esc([q.resolution, q.playing ? t("頁面播放中") : ""].filter(Boolean).join(" · ")) + "</span></button>";
        }).join("") + "</div>" +
        '<span>' + t("在抖音頁面上切換清晰度，這裡會跟著標出正在播的那一路。") + "</span>";
      if (exist && status(exist) === "rec") h += "<span>" + t("這個直播間已經在錄製。") + "</span>";
      else h += '<div style="display:flex;gap:8px;flex-wrap:wrap"><button class="btn sm red" data-act="dyrec">' + ic("rec") + t("錄製這個畫質") + '</button><button class="btn sm ghost" data-act="dyadd">' + t(exist ? "更新網址" : "只加入清單") + "</button></div>";
    }
    h += "</div>";
    // 重繪會把清單捲回最上面，有新回報時保持原本的捲動位置
    var ql = el.querySelector(".qlist"), qtop = ql ? ql.scrollTop : 0;
    el.innerHTML = h;
    ql = el.querySelector(".qlist"); if (ql) ql.scrollTop = qtop;
    placeDy(true);
  }
  // 同一路串流的網址每次簽名不同，用去掉參數的路徑辨識
  function skey(url) { return String(url).split("?")[0]; }
  function curCands() { return S.dy.rid ? S.dy.cands[S.dy.rid] : null; }
  // 使用者選過就用選的；沒選過時跟著頁面正在播的那一路，再不然用最高畫質
  function pickOf(c) {
    var k = S.dy.pick[c.webRid];
    var hit = k && c.streams.filter(function (q) { return skey(q.url) === k; })[0];
    return hit || c.streams.filter(function (q) { return q.playing; })[0] || c.streams[0];
  }
  function findRoomFor(c) {
    var keys = c.streams.map(function (q) { return skey(q.url); });
    return S.rooms.filter(function (r) { return (c.webRid && r.webRid === c.webRid) || keys.indexOf(skey(r.url)) >= 0; })[0];
  }
  var dyRO = null;
  function placeDy(open) {
    var web = $("dyweb"); if (!web) return;
    var send = function () {
      // 對話框永遠在最上層：原生的子 WebView 會蓋住它，對話框開著時不顯示。
      // 這個回呼是非同步的，可能在對話框打開之後才到，所以在這裡擋，關掉對話框時 renderModal 會再顯示
      if (open && S.modal) return;
      var b = web.getBoundingClientRect();
      var args = { bounds: { x: b.left, y: b.top, w: b.width, h: b.height } };
      // 從新增對話框貼的直播間網址，打開側欄時跳過去
      if (open && S.dy.goto) { args.url = S.dy.goto; S.dy.goto = null; }
      invoke(open ? "douyin_open" : "douyin_bounds", args).catch(function (e) { toast(t("無法打開選取直播：{err}", { err: e })); });
      open = false;
    };
    if (dyRO) dyRO.disconnect();
    dyRO = new ResizeObserver(send);
    dyRO.observe(web);
  }
  window.addEventListener("resize", function () { if (S.dy.open) placeDy(false); });

  // ---------------------------------------------------------------- 對話框
  function sw(id, on) { return '<button class="sw" role="switch" aria-checked="' + on + '" data-act="sw" data-k="' + id + '" aria-label="' + t("切換") + '"></button>'; }
  function renderModal() {
    var sc = $("scrim");
    sc.hidden = !S.modal;
    // 原生的抖音子 WebView 永遠在最上層，對話框打開時先藏起來
    if (S.dy.open) invoke(S.modal ? "douyin_hide" : "douyin_open", S.modal ? {} : { bounds: dyBounds() }).catch(function () {});
    if (!S.modal) { sc.innerHTML = ""; return; }
    var h = '<div class="sheet glass" role="dialog" aria-modal="true" style="position:relative"><button class="iconbtn x" data-act="close" aria-label="' + t("關閉") + '">' + ic("x") + "</button>";
    if (S.modal === "add") {
      h += "<h2>" + t("新增直播間") + "</h2>" +
        '<label class="lbl" for="addurl">' + t("直播間網址，或抖音 App「分享」複製出來的文字") + '<textarea id="addurl" rows="3" spellcheck="false" placeholder="https://live.douyin.com/123456">' + esc(S.addText) + "</textarea></label>";
      if (S.addErr) h += '<p class="err">' + esc(S.addErr) + "</p>";
      h += '<p class="note">' + t("會在右側的「選取直播」打開這個直播間。抓到各種畫質後，在下方選一個再開始錄製。需要登入時直接在裡面登入，登入狀態只存在這台電腦的系統 WebView 裡。") + "</p>" +
        '<div class="acts"><button class="btn primary" data-act="goto">' + ic("link") + t("前往直播間") + "</button></div>";
    } else if (S.modal === "settings") {
      var s = S.settings;
      var mat = t({ "liquid-glass": "液態玻璃（macOS 26）", vibrancy: "毛玻璃（vibrancy）", mica: "Mica（Windows 11）", none: "無，使用純色底" }[S.material] || S.material);
      var lang = s.lang === "en" || s.lang === "zh-Hant" ? s.lang : "system";
      // 範例樹用虛構的直播間名稱
      var demo = t("夜貓電台");
      h += "<h2>" + t("設定") + '</h2><div class="set">' +
        '<div class="sr"><div class="t"><b>' + t("語言") + "</b><small>" + t("跟隨系統時，系統是英文就用英文，其他語言都用正體中文。") + "</small>" +
        '<div class="tabs" role="tablist" style="margin-top:6px">' + [["system", "跟隨系統"], ["zh-Hant", "正體中文"], ["en", "English"]].map(function (o) {
          // 語言名稱用它自己的語言寫，不跟著翻
          var label = o[0] === "system" ? t(o[1]) : o[1];
          return '<button role="tab" aria-selected="' + (lang === o[0]) + '" data-act="lang" data-v="' + o[0] + '">' + label + "</button>";
        }).join("") + "</div></div></div>" +
        '<div class="sr"><div class="t"><b>' + t("分段長度") + "</b><small>" + t("在影片關鍵幀切分，所以每段實際長度會差幾秒。") + '</small><div class="range"><input id="seglen" type="range" min="5" max="60" step="5" value="' + s.segMin + '" aria-label="' + t("分段長度") + '"><output id="segout">' + t("約 {n} 分鐘", { n: s.segMin }) + "</output></div></div></div>" +
        '<div class="sr"><div class="t"><b>' + t("寫檔緩衝") + "</b><small>" + t("錄到的資料先留在記憶體，攢滿才一次寫進磁碟，減少頻繁寫入。每路的大小從下一次錄製開始生效。正常結束程式時會先寫完；被強制結束時，每路最多遺失一個緩衝的資料。") + "</small>" +
        '<div class="range"><input id="bufmb" type="range" min="4" max="64" step="4" value="' + s.bufferMb + '" aria-label="' + t("每路錄製的寫檔緩衝") + '"><output id="bufout">' + t("每路 {n} MB", { n: s.bufferMb }) + "</output></div>" +
        '<div class="range"><input id="buftotal" type="range" min="64" max="256" step="32" value="' + s.bufferTotalMb + '" aria-label="' + t("所有錄製加起來的寫檔緩衝") + '"><output id="buftotalout">' + t("總計 {n} MB", { n: s.bufferTotalMb }) + "</output></div></div></div>" +
        '<div class="sr"><div class="t"><b>' + t("錄製結束後自動合併") + "</b><small>" + t("直播結束、錄製中斷或手動停止時，把分段合成一個檔案，放在 merged/。只重新封裝，不重新編碼。") + "</small></div>" + sw("autoMerge", s.autoMerge) + "</div>" +
        '<div class="sr"><div class="t"><b>' + t("合併後保留分段") + "</b><small>" + t("關掉的話，合併成功後會刪除 clips/ 裡的分段。") + "</small></div>" + sw("keepClips", s.keepClips) + "</div>" +
        '<div class="sr"><div class="t"><b>' + t("輸出位置") + "</b><small>" + t("每個直播間一個資料夾，裡面每次錄製一個以「年-月-日-時-分」命名的資料夾。沒有名稱的直播間放在「未命名直播間」底下。") + "</small>" +
        '<div style="display:flex;gap:8px;margin-top:6px"><input class="inp" id="root" value="' + esc(s.root) + '" aria-label="' + t("輸出位置") + '" style="font-family:var(--f-mono);font-size:12.5px"><button class="btn sm ghost" data-act="pick">' + t("選擇…") + "</button></div>" +
        '<pre class="tree" style="margin-top:8px">' + esc(s.root) + "/\n└─ " + demo + "/\n   └─ 2026-10-25-23-12/\n      ├─ clips/\n      ├─ cover.jpg\n      └─ merged/" + demo + "_2026-10-25-23-12.mp4</pre></div></div>" +
        '<div class="sr"><div class="t"><b>' + t("同時錄製超過 2 個時提醒") + "</b><small>" + t("抖音可能把同一個 IP 的大量拉流當成異常而暫時封鎖。SupaLive 本身不限制數量。") + "</small></div>" + sw("warnMulti", s.warnMulti) + "</div>" +
        '<div class="sr"><div class="t"><b>' + t("省電模式") + "</b><small>" + t("關掉玻璃模糊效果，改用不透明底色。macOS 和 Windows 效果相同。") + "</small></div>" + sw("eco", s.eco) + "</div>" +
        '<div class="sr"><div class="t"><b>' + t("視窗材質") + "</b><small>" + esc(mat) + (S.privateApi ? "" : t("。這個版本關閉了 macOS 私有 API（上架版設定），視窗不透明。")) + "</small></div></div>" +
        "</div>";
    } else if (S.modal.type === "warn") {
      var r = room(S.modal.id);
      h += "<h2>" + t("要開始第 {n} 路錄製嗎？", { n: nRec() + 1 }) + '</h2><div class="banner">' + ic("warn") + "<p>" + t("已經有 {n} 個直播間在錄製。同時錄製超過 2 個可能觸發抖音風控，所有拉流都可能被暫時封鎖。", { n: nRec() }) + "</p></div>" +
        '<p class="note">' + t("SupaLive 不會擋你。如果之後開始斷線，停掉幾路再重試。") + "</p>" +
        '<div class="acts"><button class="btn ghost" data-act="close">' + t("取消") + '</button><button class="btn red" data-act="forcestart" data-id="' + r.id + '">' + ic("rec") + t("仍要錄製「{name}」", { name: esc(title(r)) }) + "</button></div>";
    } else if (S.modal.type === "rmrec") {
      var mr = room(S.modal.id), mx = mr && mr.recordings[S.modal.i];
      if (!mx) { S.modal = null; return renderModal(); }
      var mgone = !presOf(mr, S.modal.i).dir;
      h += "<h2>" + t("刪除這次錄製？") + '</h2><p class="note">' + (mgone
        ? t("「{name}」{time} 的錄製資料夾已經找不到，只會從清單移除。", { name: esc(title(mr)), time: when(mx.started) })
        : t(S.modal.permanent ? "「{name}」{time} 的錄製，整個資料夾會永久刪除：" : "「{name}」{time} 的錄製，整個資料夾會移到{bin}：", { name: esc(title(mr)), time: when(mx.started), bin: bin() })) + "</p>" +
        '<pre class="tree selectable">' + esc(mx.dir) + "</pre>" + (mgone ? "" : permBox()) +
        '<div class="acts"><button class="btn ghost" data-act="close">' + t("取消") + '</button><button class="btn danger" data-act="dormrec" data-id="' + mr.id + '" data-i="' + S.modal.i + '">' + ic("trash") + delLabel(mgone) + "</button></div>";
    } else if (S.modal.type === "remove") {
      var rr = room(S.modal.id);
      // 資料夾都已經不在的錄製不用刪，算剩下的
      var nr = rr.recordings.filter(function (x, i) { return presOf(rr, i).dir; }).length;
      h += "<h2>" + t("刪除「{name}」？", { name: esc(title(rr)) }) + '</h2><p class="note">' + (nr
        ? t(S.modal.permanent ? "這個直播間和它的 {n} 次錄製會一起永久刪除。" : "這個直播間和它的 {n} 次錄製會一起移到{bin}。", { n: nr, bin: bin() })
        : t("這個直播間沒有找得到的錄製檔案，只會從清單移除。")) + "</p>" + (nr ? permBox() : "") +
        '<div class="acts"><button class="btn ghost" data-act="close">' + t("取消") + '</button><button class="btn danger" data-act="doremove" data-id="' + rr.id + '">' + ic("trash") + delLabel(!nr) + "</button></div>";
    } else if (S.modal.type === "closing") {
      h = '<div class="sheet glass" role="dialog" aria-modal="true"><h2>' + t("正在封裝錄製…") + '</h2><p class="note">' + t(S.settings.autoMerge ? "停止所有錄製、寫出緩衝、封好最後一個分段並合併，完成後自動關閉。" : "停止所有錄製、寫出緩衝、封好最後一個分段，完成後自動關閉。") + "</p>";
    }
    h += "</div>";
    sc.innerHTML = h;
    var pm = $("perm");
    if (pm) pm.onchange = function () { S.modal.permanent = pm.checked; renderModal(); var p2 = $("perm"); if (p2) p2.focus(); };
    var ta = $("addurl");
    if (ta) {
      ta.oninput = function () { S.addText = ta.value; var er = sc.querySelector(".err"); if (er) { S.addErr = ""; er.remove(); } };
      ta.onkeydown = function (e) { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); goto(); } };
    }
    [["bufmb", "bufferMb", "bufout", "每路 {n} MB"], ["buftotal", "bufferTotalMb", "buftotalout", "總計 {n} MB"]].forEach(function (b) {
      var el = $(b[0]);
      if (el) el.oninput = function () { S.settings[b[1]] = +el.value; $(b[2]).textContent = t(b[3], { n: el.value }); saveSettings(); };
    });
    var sl = $("seglen");
    if (sl) sl.oninput = function () { S.settings.segMin = +sl.value; $("segout").textContent = t("約 {n} 分鐘", { n: sl.value }); saveSettings(); };
    var rt = $("root");
    if (rt) rt.onchange = function () { if (rt.value.trim()) { S.settings.root = rt.value.trim(); saveSettings(); renderModal(); } };
  }
  // 刪除對話框的「永久刪除」，預設不勾
  function permBox() {
    return '<label class="chk"><input type="checkbox" id="perm"' + (S.modal.permanent ? " checked" : "") + "> " + t("永久刪除，不放進{bin}", { bin: bin() }) + "</label>" +
      (S.modal.permanent ? '<p class="err">' + t("永久刪除後無法復原。") + "</p>" : "");
  }
  function delLabel(listOnly) { return listOnly ? t("從清單移除") : S.modal.permanent ? t("永久刪除") : t("移到{bin}", { bin: bin() }); }
  function dyBounds() { var w = $("dyweb"); if (!w) return { x: 0, y: 0, w: 1, h: 1 }; var b = w.getBoundingClientRect(); return { x: b.left, y: b.top, w: b.width, h: b.height }; }

  // ---------------------------------------------------------------- 動作
  function toast(t) { var el = document.createElement("div"); el.className = "toast glass"; el.textContent = t; $("toasts").appendChild(el); setTimeout(function () { el.remove(); }, 3600); }
  function fail(e) { toast(String(e)); }
  function saveSettings() { applyClasses(); invoke("save_settings", { settings: S.settings }).catch(fail); renderMain(); }

  function start(id, force) {
    if (!force && S.settings.warnMulti && nRec() >= 2) { S.modal = { type: "warn", id: id }; renderModal(); return; }
    S.modal = null; renderModal();
    invoke("start_recording", { id: id }).then(function () { toast(t("開始錄製「{name}」", { name: title(room(id)) })); return refresh(); }).catch(fail);
  }
  function addRoom(info, rec) {
    return invoke("add_room", { room: info }).then(function (r) {
      return refresh().then(function () {
        if (rec) start(r.id); else toast(t("已加入「{name}」", { name: title(r) }));
        return r;
      });
    }).catch(fail);
  }

  // 新增直播間：側欄跳到貼上的網址。這裡只粗略檢查，網址由 Rust 端再找一次
  function goto() {
    var text = S.addText.trim();
    if (!/https?:\/\/\S*douyin\.com/i.test(text)) {
      S.addErr = t("找不到抖音直播間的網址。請貼上 live.douyin.com 的網址，或抖音 App 分享出來的連結。");
      return renderModal();
    }
    S.addText = ""; S.addErr = ""; S.dy.goto = text; S.dy.open = true; S.modal = null;
    renderModal(); renderAll();
  }

  var win = T.window.getCurrentWindow();
  document.addEventListener("click", function (e) {
    var b = e.target.closest("[data-act]"); if (!b) return;
    var a = b.dataset.act, id = b.dataset.id != null ? +b.dataset.id : null;
    switch (a) {
      case "win-min": win.minimize(); break;
      case "win-max": win.toggleMaximize(); break;
      case "win-close": win.close(); break;
      case "home": S.view = "grid"; S.sel = null; renderAll(); break;
      case "open": S.view = "player"; S.sel = id; S.recIdx = null; S.p.clip = null; renderAll(); $("main").scrollTop = 0; break;
      case "openrec": S.view = "player"; S.sel = id; S.recIdx = +b.dataset.i; S.p.clip = null; renderAll(); $("main").scrollTop = 0; break;
      case "fold": S.fold[id] = !S.fold[id]; renderMain(); break;
      case "more": S.more[id] = true; renderMain(); break;
      case "rmrec": S.modal = { type: "rmrec", id: id, i: +b.dataset.i }; renderModal(); break;
      case "dormrec":
        var perm1 = !!S.modal.permanent, i1 = +b.dataset.i;
        S.modal = null; renderModal();
        // 正在看這一次的話回到最近一次
        if (S.sel == id && S.recIdx === i1) S.recIdx = null;
        invoke("remove_recording", { id: id, index: i1, permanent: perm1 }).then(function () { toast(perm1 ? t("已永久刪除") : t("已移到{bin}", { bin: bin() })); return refresh(); }).catch(function (e) { fail(e); refresh(); });
        break;
      case "start": start(id); break;
      case "forcestart": start(id, true); break;
      case "stop": invoke("stop_recording", { id: id }).then(function () { toast(t("正在停止「{name}」，封裝最後一個分段", { name: title(room(id)) })); }).catch(fail); break;
      case "merge": invoke("merge_recording", { id: id, index: b.dataset.i != null ? +b.dataset.i : null }).then(function () { toast(t("「{name}」已合併", { name: title(room(id)) })); }).catch(fail); break;
      case "remove": S.modal = { type: "remove", id: id }; renderModal(); break;
      case "doremove":
        var perm2 = !!S.modal.permanent;
        S.modal = null; renderModal();
        if (S.sel == id) { S.view = "grid"; S.sel = null; }
        invoke("remove_room", { id: id, permanent: perm2 }).then(function () { toast(perm2 ? t("已永久刪除") : t("已移到{bin}", { bin: bin() })); return refresh(); }).catch(function (e) { fail(e); refresh(); });
        break;
      case "reveal": invoke("reveal", { path: b.dataset.path }).catch(fail); break;
      case "clip": S.p.clip = b.dataset.path; renderMain(); break;
      case "add": S.modal = "add"; S.addErr = ""; renderModal(); setTimeout(function () { var ta = $("addurl"); if (ta) ta.focus(); }, 0); break;
      case "settings": S.modal = "settings"; renderModal(); break;
      case "lang": S.settings.lang = b.dataset.v; applyLang(); saveSettings(); renderAll(); break;
      case "close": S.modal = null; renderModal(); break;
      case "goto": goto(); break;
      // 在選取直播打開這個直播間；抓到新的拉流網址後，用「更新網址」或「錄製這個畫質」換掉舊的
      case "relink": var rr = room(id); if (rr && rr.webRid) { S.dy.goto = "https://live.douyin.com/" + rr.webRid; S.dy.open = true; renderAll(); } break;
      case "dytoggle": S.dy.open = !S.dy.open; renderAll(); break;
      case "dyback": invoke("douyin_back").catch(fail); break;
      case "dyreload": invoke("douyin_reload").catch(fail); break;
      case "dypick": var pc = curCands(); if (pc) { S.dy.pick[pc.webRid] = b.dataset.k; renderDy(); } break;
      case "dyrec": case "dyadd":
        var c = curCands(); if (!c) break;
        var q = pickOf(c), hdr = [["Referer", "https://live.douyin.com/"]];
        if (c.userAgent) hdr.push(["User-Agent", c.userAgent]);
        addRoom({ url: q.url, headers: hdr, anchor: c.anchor || "", name: c.anchor || "", webRid: c.webRid || null }, a === "dyrec").then(renderDy);
        break;
      case "pp": var v = $("pv"); if (v) { if (v.paused) { if (player) player.userPaused = false; v.play(); } else { if (player) { player.userPaused = true; player.follow = false; } v.pause(); } } break;
      case "golive": var pv = $("pv"), rg = player && player.range(); if (pv && rg) { player.follow = true; player.userPaused = false; pv.currentTime = player.liveTime(); pv.play(); } break;
      case "mute": S.p.muted = !S.p.muted; var mv = $("pv"); if (mv) mv.muted = S.p.muted; syncMute(); break;
      case "pick":
        T.dialog.open({ directory: true, defaultPath: S.settings.root }).then(function (dir) { if (dir) { S.settings.root = dir; saveSettings(); renderModal(); } }).catch(fail);
        break;
      case "sw": var k = b.dataset.k; S.settings[k] = !S.settings[k]; b.setAttribute("aria-checked", S.settings[k]); saveSettings(); renderSide(); break;
    }
  });
  $("scrim").addEventListener("click", function (e) { if (e.target === this && !(S.modal && S.modal.type === "closing")) { S.modal = null; renderModal(); } });
  document.addEventListener("keydown", function (e) { if (e.key === "Escape" && S.modal && !(S.modal.type === "closing")) { S.modal = null; renderModal(); } });

  // ---------------------------------------------------------------- 即時更新
  function bind(k) {
    var p = k.split(":"), r = p[1] && room(p[1]);
    switch (p[0]) {
      case "dur": return r ? dur(elapsed(r)) : "";
      case "size": return r ? bytes(recBytes(r)) : "";
      case "seg": return r ? t("第 {n} 段", { n: lastRec(r).clips.length + 1 }) : "";
      case "kbps": var k2 = r && kbps(r); return k2 ? (k2 / 1000).toFixed(1) + " Mb/s" : "—";
      // 幀率由錄製核心依影片時間戳每秒算一次；接近整數時不顯示小數
      case "fps": var f = r && S.live[r.id] && S.live[r.id].fps; if (!f) return "—"; return (Math.abs(f - Math.round(f)) < 0.05 ? Math.round(f) : f.toFixed(1)) + " fps";
      case "seghint": if (!r) return ""; var left = segLen() - (elapsed(r) % segLen()); return t("約 {n} 分鐘後在關鍵幀切下一段", { n: Math.ceil(left / 60) });
      case "ptime": var v = $("pv"), rg = player && player.range(); if (!v || !rg) return "—"; return dur(v.currentTime - rg[0]) + " / " + dur(rg[1] - rg[0]);
      case "golive": var v2 = $("pv"), rg2 = player && player.range(); if (!v2 || !rg2) return t("● 直播"); var lag = rg2[1] - v2.currentTime; return lag < player.lag() + 3 ? t("● 直播") : t("回到直播 −{t}", { t: dur(lag) });
      case "lat": var v3 = $("pv"), rg3 = player && player.range(); if (!v3 || !rg3) return "LIVE"; var l3 = rg3[1] - v3.currentTime; return l3 < player.lag() + 3 ? "LIVE" : t("延遲 {t}", { t: dur(l3) });
    }
  }
  function tick() {
    document.querySelectorAll("[data-b]").forEach(function (el) {
      var v = bind(el.dataset.b);
      if (v != null && el.textContent !== v) el.textContent = v;
      if (el.dataset.b === "golive") el.classList.toggle("at", v === t("● 直播"));
    });
    document.querySelectorAll("[data-w]").forEach(function (el) {
      var p = el.dataset.w.split(":"), w = 0;
      if (p[0] === "segp") { var r = room(p[1]); if (r) w = ((elapsed(r) % segLen()) / segLen()) * 100; }
      if (p[0] === "pfill") { var v = $("pv"), rg = player && player.range(); if (v && rg) w = ((v.currentTime - rg[0]) / (rg[1] - rg[0] || 1)) * 100; }
      el.style.width = Math.max(0, Math.min(100, w)) + "%";
    });
    var kn = document.querySelector('[data-l="pknob"]');
    if (kn) { var v = $("pv"), rg = player && player.range(); kn.style.left = v && rg ? Math.max(0, Math.min(100, ((v.currentTime - rg[0]) / (rg[1] - rg[0] || 1)) * 100)) + "%" : "100%"; }
  }
  setInterval(tick, 500);
  setupResizers();

  function applyClasses() {
    var c = document.documentElement.classList;
    c.toggle("native", S.material !== "none");
    c.toggle("mat-mica", S.material === "mica");
    c.toggle("eco", !!(S.settings && S.settings.eco));
    c.add("pf-" + S.platform);
  }

  function refresh() {
    return invoke("get_state").then(function (st) {
      S.settings = st.settings; S.rooms = st.rooms; S.covers = st.covers || {}; S.present = st.present || {}; S.platform = st.platform; S.material = st.material; S.privateApi = st.privateApi;
      applyClasses(); applyLang();
      renderSide(); renderMain();
      if (S.dy.open && curCands() && !S.modal) renderDy();
    });
  }
  function renderAll() { renderSide(); renderMain(); renderDy(); if (S.modal) renderModal(); }
  // 切回 App 時重新檢查檔案還在不在；沒有變化就不重繪，免得正在播的檔案被打斷
  function recheckFiles() {
    if (!S.settings) return;
    invoke("get_state").then(function (st) {
      if (JSON.stringify(st.present || {}) !== JSON.stringify(S.present)) refresh();
    }).catch(function () {});
  }
  window.addEventListener("focus", recheckFiles);
  document.addEventListener("visibilitychange", function () { if (!document.hidden) recheckFiles(); });

  var refreshTimer = null;
  listen("rooms-changed", function () { clearTimeout(refreshTimer); refreshTimer = setTimeout(refresh, 80); });
  listen("rec-event", function (e) {
    var p = e.payload, r = room(p.roomId), name = r ? title(r) : t("未命名直播間");
    if (p.kind === "stats") {
      var l = S.live[p.roomId] || (S.live[p.roomId] = { bytes: 0, samples: [] });
      l.bytes = p.bytes; l.fps = p.fps; l.samples.push([Date.now(), p.bytes]);
      while (l.samples.length > 8) l.samples.shift();
      return;
    }
    if (p.kind === "clip" && r && status(r) === "rec") toast(t("「{name}」第 {n} 段已存好", { name: name, n: lastRec(r).clips.length + 1 }));
    if (p.kind === "merged") toast(t("「{name}」已合併到 merged/", { name: name }));
    if (p.kind === "disconnected") toast(t("「{name}」{reason}（{a}/{b}），5 秒後重試", { name: name, reason: p.reason, a: p.failures, b: p.max }));
    if (p.kind === "warning") toast(t("「{name}」{msg}", { name: name, msg: p.message }));
  });
  listen("douyin-nav", function (e) {
    S.dy.url = String(e.payload);
    var m = /live\.douyin\.com\/(\d+)/.exec(S.dy.url);
    S.dy.rid = m ? m[1] : null;
    // 之前來過的直播間，後端可能還留著抓到的畫質
    if (S.dy.rid && !S.dy.cands[S.dy.rid]) {
      invoke("douyin_streams", { webRid: S.dy.rid }).then(function (c) {
        if (c && c.webRid) { S.dy.cands[c.webRid] = c; if (S.dy.open && !S.modal && S.dy.rid === c.webRid) renderDy(); }
      }).catch(function () {});
    }
    if (S.dy.open) renderDy();
  });
  // 後端暫存同一個直播間的各種畫質，每次有新的就送整份清單
  listen("douyin-streams", function (e) {
    var c = e.payload; if (!c || !c.webRid) return;
    S.dy.cands[c.webRid] = c;
    // 抖音是單頁應用，在頁面裡切直播間不一定有 douyin-nav，以最新的回報為準
    S.dy.rid = c.webRid;
    if (c.pageUrl) S.dy.url = c.pageUrl;
    if (S.dy.open && !S.modal) renderDy();
  });
  listen("closing", function () { S.modal = { type: "closing" }; renderModal(); });

  refresh().catch(function (e) { document.body.textContent = t("SupaLive 啟動失敗：{err}", { err: e }); });
})();
