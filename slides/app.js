(() => {
  "use strict";

  const deck = document.querySelector("#deck");
  const slides = [...document.querySelectorAll(".slide")];
  const counter = document.querySelector("#slide-counter");
  const progressBar = document.querySelector("#progress-bar");
  const dotsContainer = document.querySelector("#slide-dots");
  const toast = document.querySelector("#toast");
  const notesDrawer = document.querySelector("#notes-drawer");
  const notesContent = document.querySelector("#notes-content");
  const backupPanel = document.querySelector("#backup-panel");
  const helpOverlay = document.querySelector("#help-overlay");
  const fullscreenButton = document.querySelector("#fullscreen-button");
  const themeButton = document.querySelector("#theme-button");
  const themeColorMeta = document.querySelector('meta[name="theme-color"]');
  const themeStorageKey = "bbcloud-theme";
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const systemLight = window.matchMedia("(prefers-color-scheme: light)");
  // Must match the handout media query in styles.css.
  const handoutMode = window.matchMedia("screen and (max-width: 760px), screen and (orientation: portrait) and (max-width: 1100px)");
  const themeOrder = ["auto", "dark", "light"];

  // "auto" follows the Mac's appearance; dark/light pin the deck regardless of the OS.
  const applyTheme = (mode, persist = true) => {
    const nextMode = themeOrder.includes(mode) ? mode : "auto";
    if (nextMode === "auto") {
      delete document.documentElement.dataset.theme;
    } else {
      document.documentElement.dataset.theme = nextMode;
    }
    const effective = nextMode === "auto" ? (systemLight.matches ? "light" : "dark") : nextMode;
    if (themeColorMeta) {
      themeColorMeta.setAttribute("content", effective === "light" ? "#f5f5f7" : "#0b0d11");
    }
    if (themeButton) {
      const nextLabel = themeOrder[(themeOrder.indexOf(nextMode) + 1) % themeOrder.length];
      themeButton.setAttribute("aria-label", `Theme: ${nextMode}. Switch to ${nextLabel}`);
      themeButton.title = `Theme: ${nextMode} (T)`;
    }
    if (persist) {
      try {
        if (nextMode === "auto") {
          window.localStorage.removeItem(themeStorageKey);
        } else {
          window.localStorage.setItem(themeStorageKey, nextMode);
        }
      } catch (_) {
        // Storage is optional; the visual toggle still works.
      }
    }
    return effective;
  };

  const currentThemeMode = () => document.documentElement.dataset.theme || "auto";

  applyTheme(currentThemeMode(), false);
  systemLight.addEventListener("change", () => applyTheme(currentThemeMode(), false));

  if (!deck || slides.length === 0) {
    return;
  }

  let current = 0;
  let scrollFrame = 0;
  let wheelAccumulator = 0;
  let wheelResetTimer = 0;
  let touchStartX = 0;
  let touchStartY = 0;
  let touchStartTime = 0;
  let toastTimer = 0;
  let programmaticScroll = false;

  const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

  const getHashIndex = () => {
    const match = window.location.hash.match(/^#slide-(\d+)$/);
    if (!match) {
      return 0;
    }
    return clamp(Number(match[1]) - 1, 0, slides.length - 1);
  };

  const getNotes = (slide) => {
    const notes = slide.querySelector(".speaker-notes");
    return notes ? notes.textContent.trim() : "No speaker notes for this slide.";
  };

  const showToast = (message) => {
    if (!toast) {
      return;
    }
    window.clearTimeout(toastTimer);
    toast.textContent = message;
    toast.classList.add("is-visible");
    toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 1800);
  };

  const updateNotes = (index) => {
    if (!notesContent) {
      return;
    }
    notesContent.textContent = getNotes(slides[index]);
  };

  const updateChrome = (index) => {
    current = clamp(index, 0, slides.length - 1);
    const displayIndex = String(current + 1).padStart(2, "0");

    if (counter) {
      counter.textContent = `${displayIndex} / ${String(slides.length).padStart(2, "0")}`;
    }
    if (progressBar) {
      progressBar.style.width = `${((current + 1) / slides.length) * 100}%`;
    }

    slides.forEach((slide, slideIndex) => {
      const isActive = slideIndex === current;
      const wasActive = slide.classList.contains("is-active");
      slide.classList.toggle("is-active", isActive);
      if (isActive && !wasActive) {
        slide.dispatchEvent(new CustomEvent("slide:enter", { bubbles: true }));
      }
      if (isActive) {
        slide.classList.add("is-seen");
        slide.setAttribute("aria-current", "page");
      } else {
        slide.removeAttribute("aria-current");
      }
    });

    document.querySelectorAll(".slide-dot").forEach((dot, dotIndex) => {
      const isActive = dotIndex === current;
      dot.classList.toggle("is-active", isActive);
      dot.setAttribute("aria-current", isActive ? "true" : "false");
    });

    updateNotes(current);
    document.title = `${displayIndex} · bb — Bitbucket Cloud CLI`;
  };

  const goTo = (requestedIndex, behavior = "smooth") => {
    const index = clamp(requestedIndex, 0, slides.length - 1);
    programmaticScroll = true;
    updateChrome(index);
    slides[index].scrollIntoView({
      behavior: reduceMotion.matches || behavior === "auto" ? "auto" : "smooth",
      block: "start",
      inline: "nearest",
    });
    try {
      window.history.replaceState(null, "", `#slide-${index + 1}`);
    } catch {
      window.location.hash = `#slide-${index + 1}`;
    }
    window.setTimeout(() => {
      programmaticScroll = false;
    }, reduceMotion.matches ? 80 : 650);
  };

  const next = () => goTo(current + 1);
  const previous = () => goTo(current - 1);

  const nearestSlide = () => {
    let nearest = 0;
    let distance = Number.POSITIVE_INFINITY;
    slides.forEach((slide, index) => {
      const rect = slide.getBoundingClientRect();
      const candidateDistance = Math.abs(rect.top);
      if (candidateDistance < distance) {
        nearest = index;
        distance = candidateDistance;
      }
    });
    return nearest;
  };

  const createDots = () => {
    if (!dotsContainer) {
      return;
    }
    dotsContainer.innerHTML = "";
    slides.forEach((slide, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "slide-dot";
      button.setAttribute("aria-label", `Go to slide ${index + 1}: ${slide.dataset.title || "slide"}`);
      button.addEventListener("click", () => goTo(index));
      dotsContainer.appendChild(button);
    });
  };

  const handleScroll = () => {
    if (scrollFrame) {
      return;
    }
    scrollFrame = window.requestAnimationFrame(() => {
      if (!programmaticScroll) {
        updateChrome(nearestSlide());
      }
      scrollFrame = 0;
    });
  };

  const handleWheel = (event) => {
    if (handoutMode.matches || event.ctrlKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
      return;
    }

    event.preventDefault();
    wheelAccumulator += event.deltaY;
    window.clearTimeout(wheelResetTimer);
    wheelResetTimer = window.setTimeout(() => {
      wheelAccumulator = 0;
    }, 260);

    if (Math.abs(wheelAccumulator) < 28) {
      return;
    }

    const direction = wheelAccumulator > 0 ? 1 : -1;
    wheelAccumulator = 0;
    if (direction > 0) {
      next();
    } else {
      previous();
    }
  };

  const handleTouchStart = (event) => {
    const touch = event.changedTouches[0];
    touchStartX = touch.clientX;
    touchStartY = touch.clientY;
    touchStartTime = Date.now();
  };

  const handleTouchEnd = (event) => {
    if (handoutMode.matches) {
      return;
    }
    const touch = event.changedTouches[0];
    const deltaX = touch.clientX - touchStartX;
    const deltaY = touch.clientY - touchStartY;
    const elapsed = Date.now() - touchStartTime;
    const isSwipe = elapsed < 700 && Math.abs(deltaY) > 45 && Math.abs(deltaY) > Math.abs(deltaX) * 1.1;
    if (!isSwipe) {
      return;
    }
    if (deltaY < 0) {
      next();
    } else {
      previous();
    }
  };

  const toggleFullscreen = async () => {
    try {
      if (!document.fullscreenElement) {
        await document.documentElement.requestFullscreen();
        showToast("Fullscreen on");
      } else {
        await document.exitFullscreen();
        showToast("Fullscreen off");
      }
    } catch {
      showToast("Fullscreen is unavailable here");
    }
  };

  const closeUtilities = () => {
    if (notesDrawer) {
      notesDrawer.hidden = true;
    }
    if (backupPanel) {
      backupPanel.hidden = true;
    }
    if (helpOverlay) {
      helpOverlay.hidden = true;
    }
  };

  const toggleUtility = (element) => {
    if (!element) {
      return;
    }
    const shouldOpen = element.hidden;
    closeUtilities();
    element.hidden = !shouldOpen;
  };

  const handleKey = (event) => {
    const target = event.target;
    const isTyping = target && ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
    if (isTyping) {
      return;
    }

    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
      case "PageDown":
      case " ":
        event.preventDefault();
        next();
        break;
      case "ArrowLeft":
      case "ArrowUp":
      case "PageUp":
        event.preventDefault();
        previous();
        break;
      case "Home":
        event.preventDefault();
        goTo(0);
        break;
      case "End":
        event.preventDefault();
        goTo(slides.length - 1);
        break;
      case "f":
      case "F":
        event.preventDefault();
        toggleFullscreen();
        break;
      case "t":
      case "T":
        event.preventDefault();
        themeButton?.click();
        break;
      case "n":
      case "N":
        event.preventDefault();
        toggleUtility(notesDrawer);
        break;
      case "b":
      case "B":
        event.preventDefault();
        toggleUtility(backupPanel);
        break;
      case "?":
        event.preventDefault();
        toggleUtility(helpOverlay);
        break;
      case "Escape":
        closeUtilities();
        break;
      default:
        break;
    }
  };

  const handleEdgeClick = (event) => {
    if (handoutMode.matches) {
      return;
    }
    if (event.target.closest("a, button, input, textarea, select, pre, code, .shot, .drawer, .help-overlay")) {
      return;
    }
    if (event.clientX < window.innerWidth * 0.12) {
      previous();
    } else if (event.clientX > window.innerWidth * 0.88) {
      next();
    }
  };

  const copyText = async (text) => {
    try {
      await navigator.clipboard.writeText(text);
      showToast("Command copied");
    } catch {
      showToast("Copy is unavailable in this browser");
    }
  };

  document.querySelectorAll("[data-action='prev']").forEach((button) => button.addEventListener("click", previous));
  document.querySelectorAll("[data-action='next']").forEach((button) => button.addEventListener("click", next));
  document.querySelectorAll("[data-close]").forEach((button) => {
    button.addEventListener("click", () => {
      const target = document.getElementById(button.dataset.close);
      if (target) {
        target.hidden = true;
      }
    });
  });
  document.querySelectorAll("[data-copy]").forEach((button) => {
    button.addEventListener("click", () => copyText(button.dataset.copy));
  });

  if (fullscreenButton) {
    fullscreenButton.addEventListener("click", toggleFullscreen);
  }
  if (themeButton) {
    themeButton.addEventListener("click", () => {
      const current = currentThemeMode();
      const nextMode = themeOrder[(themeOrder.indexOf(current) + 1) % themeOrder.length];
      const effective = applyTheme(nextMode);
      showToast(nextMode === "auto" ? `Theme: auto (${effective})` : `Theme: ${nextMode}`);
    });
  }
  const helpButton = document.querySelector("#help-button");
  if (helpButton) {
    helpButton.addEventListener("click", () => toggleUtility(helpOverlay));
  }

  let idleTimer = 0;
  const wake = () => {
    document.body.classList.remove("is-idle");
    window.clearTimeout(idleTimer);
    idleTimer = window.setTimeout(() => document.body.classList.add("is-idle"), 2500);
  };
  window.addEventListener("mousemove", wake, { passive: true });
  window.addEventListener("touchstart", wake, { passive: true });
  wake();

  deck.addEventListener("wheel", handleWheel, { passive: false });
  deck.addEventListener("scroll", handleScroll, { passive: true });
  deck.addEventListener("touchstart", handleTouchStart, { passive: true });
  deck.addEventListener("touchend", handleTouchEnd, { passive: true });
  deck.addEventListener("click", handleEdgeClick);
  window.addEventListener("keydown", handleKey);
  window.addEventListener("hashchange", () => goTo(getHashIndex(), "auto"));
  window.addEventListener("resize", () => updateChrome(nearestSlide()));

  createDots();
  const initialIndex = getHashIndex();
  updateChrome(initialIndex);
  window.requestAnimationFrame(() => {
    slides[initialIndex].scrollIntoView({ behavior: "auto", block: "start" });
  });

  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(() => document.body.classList.add("fonts-ready"));
  }
})();
