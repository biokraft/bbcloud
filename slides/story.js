(() => {
  "use strict";

  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  // Must match the handout media query in styles.css.
  const handoutMode = window.matchMedia("screen and (max-width: 760px), screen and (orientation: portrait) and (max-width: 1100px)");
  const instant = () => reduceMotion.matches || handoutMode.matches;

  // One real pull request, anonymised: every root inline thread as [file index, theme index], in the
  // order Bitbucket returns them. Theme -1 is a comment its author deleted; theme 8 is a design question.
  // Clustered by keyword from `bb pr view --json` — approximate, not hand-labelled.
  const THREADS = [[0,0],[1,2],[2,2],[2,0],[3,0],[4,0],[5,0],[6,8],[7,0],[8,6],[9,2],[10,2],[11,8],[12,0],[13,0],[14,8],[15,-1],[14,1],[16,0],[14,8],[14,8],[14,3],[17,6],[14,8],[18,0],[14,8],[19,1],[14,8],[12,8],[14,3],[20,1],[21,-1],[21,-1],[21,-1],[20,7],[21,8],[22,8],[23,4],[24,8],[25,8],[25,8],[14,1],[26,8],[27,4],[28,1],[29,8],[28,7],[30,8],[31,8],[28,1],[28,8],[32,5],[28,1],[33,8],[31,3],[32,5],[34,4]];
  const FILE_COUNT = 35;
  const FIGHT_ORDER = [0, 1, 2, 3, 4, 5, 6, 7, 8, -1];
  const STEP_MS = 650;

  const count = (theme) => THREADS.filter(([, t]) => t === theme).length;

  const buildMap = (map) => {
    const groups = Array.from({ length: FILE_COUNT }, () => {
      const group = document.createElement("span");
      group.className = "tm-file";
      return group;
    });
    THREADS.forEach(([file, theme], index) => {
      const dot = document.createElement("span");
      dot.className = "tm-dot";
      dot.dataset.t = String(theme);
      dot.style.setProperty("--d", `${index * 18}ms`);
      groups[file].appendChild(dot);
    });
    groups.forEach((group) => map.appendChild(group));
  };

  const formatNumber = (value, decimals) =>
    value.toLocaleString("en-US", { minimumFractionDigits: decimals, maximumFractionDigits: decimals });

  const runCounters = (slide) => {
    slide.querySelectorAll("[data-count]").forEach((el) => {
      const target = Number(el.dataset.count);
      const decimals = Number(el.dataset.decimals || 0);
      if (instant()) {
        el.textContent = formatNumber(target, decimals);
        return;
      }
      const start = performance.now();
      const duration = Number(el.dataset.duration || 1200);
      const tick = (now) => {
        const progress = Math.min(1, (now - start) / duration);
        const eased = 1 - Math.pow(1 - progress, 3);
        el.textContent = formatNumber(target * eased, decimals);
        if (progress < 1) {
          window.requestAnimationFrame(tick);
        }
      };
      window.requestAnimationFrame(tick);
    });
  };

  const timers = new WeakMap();

  const clearTimers = (slide) => {
    (timers.get(slide) || []).forEach((id) => {
      window.clearTimeout(id);
      window.clearInterval(id);
    });
    timers.set(slide, []);
  };

  const later = (slide, fn, ms) => {
    const id = window.setTimeout(fn, ms);
    timers.get(slide).push(id);
  };

  const focusTheme = (map, legend, theme) => {
    map.querySelectorAll(".tm-dot").forEach((dot) => dot.classList.toggle("is-focus", dot.dataset.t === theme));
    map.classList.toggle("has-focus", theme !== null);
    legend?.querySelectorAll("[data-theme]").forEach((row) => row.classList.toggle("is-on", row.dataset.theme === theme));
  };

  const runThemes = (slide, map) => {
    const legend = slide.querySelector(".tm-legend");
    const themes = [...(legend?.querySelectorAll("[data-theme]") || [])].map((row) => row.dataset.theme);
    if (themes.length === 0) {
      return;
    }
    let index = 0;
    focusTheme(map, legend, themes[0]);
    if (instant()) {
      return;
    }
    const id = window.setInterval(() => {
      index = (index + 1) % themes.length;
      focusTheme(map, legend, themes[index]);
    }, 1700);
    timers.get(slide).push(id);
    legend.querySelectorAll("[data-theme]").forEach((row) => {
      row.onmouseenter = () => {
        clearTimers(slide);
        focusTheme(map, legend, row.dataset.theme);
      };
    });
  };

  const setHp = (slide, value) => {
    const hp = slide.querySelector(".hp");
    if (!hp) {
      return;
    }
    const total = Number(hp.dataset.total || THREADS.length);
    hp.querySelector(".hp-fill").style.width = `${(value / total) * 100}%`;
    hp.querySelector(".hp-n").textContent = String(value);
    hp.classList.toggle("is-low", value / total < 0.25);
    hp.classList.toggle("is-zero", value === 0);
  };

  const runFight = (slide, map) => {
    const dots = [...map.querySelectorAll(".tm-dot")];
    const steps = [...slide.querySelectorAll("[data-step]")];
    dots.forEach((dot) => dot.classList.remove("is-fixed", "is-replied", "is-gone"));
    steps.forEach((step) => step.classList.remove("is-on"));
    let remaining = THREADS.length;
    setHp(slide, remaining);

    const apply = (theme, stepIndex) => {
      const cls = theme === -1 ? "is-gone" : theme === 8 ? "is-replied" : "is-fixed";
      dots.filter((dot) => dot.dataset.t === String(theme)).forEach((dot) => dot.classList.add(cls));
      steps.filter((step) => step.dataset.step === String(stepIndex)).forEach((step) => step.classList.add("is-on"));
      remaining -= count(theme);
      setHp(slide, remaining);
    };

    if (instant()) {
      FIGHT_ORDER.forEach(apply);
      return;
    }
    FIGHT_ORDER.forEach((theme, stepIndex) => later(slide, () => apply(theme, stepIndex), 900 + stepIndex * STEP_MS));
  };

  document.querySelectorAll(".thread-map").forEach(buildMap);

  document.addEventListener("slide:enter", (event) => {
    const slide = event.target;
    clearTimers(slide);
    runCounters(slide);
    slide.querySelectorAll(".thread-map").forEach((map) => {
      const mode = map.dataset.mode;
      if (mode === "themes") {
        runThemes(slide, map);
      } else if (mode === "fight") {
        runFight(slide, map);
      }
    });
  });

  // A slide that stays active keeps its timers; one that leaves stops them.
  document.addEventListener("slide:enter", (event) => {
    document.querySelectorAll(".slide").forEach((slide) => {
      if (slide !== event.target) {
        clearTimers(slide);
      }
    });
  });

  // A handout is read, not presented: show where each animation ends.
  if (handoutMode.matches) {
    document.querySelectorAll(".slide").forEach((slide) => {
      clearTimers(slide);
      runCounters(slide);
    });
    document.querySelectorAll('.thread-map[data-mode="fight"]').forEach((map) => runFight(map.closest(".slide"), map));
  }

  // app.js activates the first slide before this script has listeners.
  document.querySelector(".slide.is-active")?.dispatchEvent(new CustomEvent("slide:enter", { bubbles: true }));
})();
