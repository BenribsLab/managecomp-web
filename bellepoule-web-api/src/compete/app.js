// Pages publiques ManageComp : onglets, filtres (nom, club), « Mon tireur »,
// mise à jour automatique pendant la compétition.
"use strict";
(function () {
  const norm = (s) => (s || "").normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().trim();
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const store = {
    get(k) { try { return JSON.parse(localStorage.getItem(k)); } catch (e) { return null; } },
    set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch (e) {} },
    del(k) { try { localStorage.removeItem(k); } catch (e) {} },
  };

  // ---------------------------------------------------- listes d'épreuves
  function cardsSearch() {
    const q = $("#cards-q");
    if (!q) return;
    q.addEventListener("input", () => {
      const v = norm(q.value);
      $$(".card, tr[data-s]").forEach((c) => c.classList.toggle("hidden", v !== "" && !norm(c.dataset.s).includes(v)));
    });
  }

  // ---------------------------------------------------- épreuve
  const main = $("main.event");
  if (!main) { document.addEventListener("DOMContentLoaded", cardsSearch); cardsSearch(); return; }
  document.documentElement.classList.add("js");
  const eventId = main.dataset.event;
  const MINE = "managecomp-mine-" + eventId;
  const TAB = "managecomp-tab-" + eventId;

  function showTab(id) {
    const tabs = $$(".tabs a");
    if (!tabs.some((a) => a.dataset.tab === id)) id = tabs.length ? tabs[0].dataset.tab : "";
    tabs.forEach((a) => a.classList.toggle("active", a.dataset.tab === id));
    $$(".tab").forEach((s) => s.classList.toggle("active", s.id === id));
    store.set(TAB, id);
  }

  // Phase en cours : onglet ouvert par défaut.
  function defaultTab() {
    const saved = store.get(TAB);
    if (saved && $("#" + saved)) return saved;
    if ($("#classement table")) return "classement";
    const current = norm(($(".formula .current") || {}).textContent);
    if (current.includes("tableau") && $("#tableaux")) return "tableaux";
    if ($("#poules")) return "poules";
    return "tireurs";
  }

  // Un tireur (ou un match) correspond-il aux filtres ?
  function matches(el, q, club) {
    const names = (el.dataset.f || "").split("|").map(norm);
    const clubs = (el.dataset.c || "").split("|");
    return (q === "" || names.some((n) => n.split(/\s+/).some((w) => w.startsWith(q)) || n.startsWith(q))) && (club === "" || clubs.includes(club));
  }

  function applyFilters() {
    const q = norm($("#q").value);
    const club = $("#club").value;
    const active = q !== "" || club !== "";
    document.documentElement.classList.toggle("filtering", active);
    $$("#content [data-f]").forEach((el) => {
      const ok = matches(el, q, club);
      el.classList.toggle("match-row", active && ok);
      // Lignes des listes et matchs : cachés ; lignes de poule : surlignées.
      if (!el.closest(".pool")) el.classList.toggle("hidden", active && !ok);
    });
    $$("#content .pool").forEach((p) => p.classList.toggle("hidden", active && !$$("[data-f]", p).some((r) => matches(r, q, club))));
    $$("#content .bouts").forEach((b) => {
      const h = b.previousElementSibling;
      const any = $$(".bout", b).some((x) => !x.classList.contains("hidden"));
      if (h && h.tagName === "H4") h.classList.toggle("hidden", active && !any);
    });
    // Sections sans aucun résultat : cachées pendant le filtrage.
    const visible = (x) => !x.closest(".hidden") && !x.closest(".trees") && (!x.closest(".pool") || x.classList.contains("match-row"));
    $$("#content .tab").forEach((s) => s.classList.toggle("hidden", active && !$$("[data-f]", s).some(visible)));
  }

  // ---------------------------------------------------- mon tireur
  function renderMine() {
    const box = $("#mine");
    const mine = store.get(MINE);
    if (!mine) {
      box.innerHTML = "";
      return;
    }
    const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
    const lines = [];
    // Poule : la dernière où il figure (tour le plus récent).
    const pools = $$("#content .pool").filter((p) => $$("[data-f]", p).some((r) => r.dataset.f === mine.f));
    const pool = pools[pools.length - 1];
    if (pool) {
      const row = $$("[data-f]", pool).find((r) => r.dataset.f === mine.f);
      const rank = row ? $$("td.res", row).pop().textContent.trim() : "";
      const where = [pool.dataset.piste && "piste " + pool.dataset.piste, pool.dataset.time && "à " + pool.dataset.time].filter(Boolean).join(" ");
      lines.push(esc(pool.dataset.title) + (where ? " — " + esc(where) : "") + (rank ? " · place " + esc(rank) : ""));
    }
    // Tableau : prochain match, ou dernier joué.
    const bouts = $$("#content .bout").filter((b) => (b.dataset.f || "").split("|").includes(mine.f));
    const next = bouts.find((b) => b.dataset.over === "0");
    if (next) {
      const other = (next.dataset.f || "").split("|").filter((n) => n !== mine.f)[0];
      const where = [next.dataset.piste && "piste " + next.dataset.piste, next.dataset.time && "à " + next.dataset.time].filter(Boolean).join(" ");
      lines.push("<span class=\"next\">Prochain match : " + esc(next.dataset.table) + (other ? " contre " + esc(other) : "") + (where ? " — " + esc(where) : "") + "</span>");
    } else if (bouts.length) {
      lines.push("Tableau : " + esc(bouts[bouts.length - 1].dataset.table));
    }
    const ranked = $$("#classement [data-f]").find((r) => r.dataset.f === mine.f);
    if (ranked) lines.push("<b>Classement final : " + esc(ranked.querySelector(".rank").textContent) + "</b>");
    box.innerHTML = "<div class=\"head\"><strong>★ " + esc(mine.f) + "</strong><button type=\"button\" id=\"mine-off\">Ne plus suivre</button></div>" +
      (lines.length ? lines.map((l) => "<div>" + l + "</div>").join("") : "<div class=\"muted\">Pas encore de poule ni de match.</div>");
    $("#mine-off").addEventListener("click", () => { store.del(MINE); renderMine(); });
  }

  function follow(el) {
    const f = (el.dataset.f || "").split("|")[0];
    if (!f || f.includes("|")) return;
    if (!confirm("Suivre " + f + " ? Sa poule, sa piste et son prochain match s'afficheront en haut de la page.")) return;
    store.set(MINE, { f: f, c: el.dataset.c });
    renderMine();
    window.scrollTo({ top: 0, behavior: "smooth" });
  }

  // ---------------------------------------------------- mise à jour
  function showUpdated() {
    const t = Number(main.dataset.updated) * 1000;
    if (!t) return;
    const d = new Date(t);
    $("#updated").textContent = "· mis à jour à " + String(d.getHours()).padStart(2, "0") + ":" + String(d.getMinutes()).padStart(2, "0");
  }

  // Mise à jour sans clignotement : seules les cases qui ont changé sont
  // remplacées (les classes posées par les filtres sont gardées).
  const JS_CLASSES = ["hidden", "match-row", "active"];
  function morph(a, b) {
    if (a.nodeType !== b.nodeType || a.nodeName !== b.nodeName) {
      a.replaceWith(b.cloneNode(true));
      return;
    }
    if (a.nodeType === 3) {
      if (a.nodeValue !== b.nodeValue) a.nodeValue = b.nodeValue;
      return;
    }
    if (a.nodeType !== 1) return;
    for (const { name, value } of Array.from(b.attributes)) {
      if (name !== "class" && a.getAttribute(name) !== value) a.setAttribute(name, value);
    }
    for (const { name } of Array.from(a.attributes)) {
      if (name !== "class" && !b.hasAttribute(name)) a.removeAttribute(name);
    }
    const cls = Array.from(b.classList).concat(Array.from(a.classList).filter((c) => JS_CLASSES.includes(c) && !b.classList.contains(c))).join(" ");
    if ((a.getAttribute("class") || "") !== cls) {
      if (cls) a.setAttribute("class", cls); else a.removeAttribute("class");
    }
    const ac = Array.from(a.childNodes);
    const bc = Array.from(b.childNodes);
    bc.forEach((n, i) => (i < ac.length ? morph(ac[i], n) : a.appendChild(n.cloneNode(true))));
    for (let i = bc.length; i < ac.length; i++) ac[i].remove();
  }

  let busy = false;
  async function refresh() {
    if (busy || document.visibilityState !== "visible") return;
    busy = true;
    try {
      // « no-cache » : le navigateur redemande avec l'empreinte de la page ;
      // rien de neuf → réponse vide du serveur.
      const r = await fetch(location.href, { cache: "no-cache" });
      if (!r.ok) return;
      const doc = new DOMParser().parseFromString(await r.text(), "text/html");
      const fresh = doc.querySelector("main.event");
      if (!fresh || fresh.dataset.updated === main.dataset.updated) return;
      main.dataset.updated = fresh.dataset.updated;
      main.dataset.live = fresh.dataset.live;
      morph($("#content"), fresh.querySelector("#content"));
      morph($(".tabs"), fresh.querySelector(".tabs"));
      showTab(store.get(TAB) || defaultTab());
      applyFilters();
      renderMine();
      showUpdated();
    } catch (e) {
    } finally {
      busy = false;
    }
  }

  // ---------------------------------------------------- démarrage
  $(".tabs").addEventListener("click", (ev) => {
    const a = ev.target.closest("a[data-tab]");
    if (!a) return;
    ev.preventDefault();
    showTab(a.dataset.tab);
  });
  $("#content").addEventListener("click", (ev) => {
    const el = ev.target.closest("tr[data-f], .cell[data-f]");
    if (el) follow(el);
  });
  $("#q").addEventListener("input", applyFilters);
  $("#club").addEventListener("change", applyFilters);
  showTab(defaultTab());
  applyFilters();
  renderMine();
  showUpdated();
  if (main.dataset.live === "1") setInterval(refresh, 5000);
})();
