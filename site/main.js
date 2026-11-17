// Theme toggle: light, dark, or the system's choice, remembered on this device.
(function () {
  const root = document.documentElement;
  const button = document.getElementById("theme");
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

  const current = () => root.dataset.theme || (systemDark.matches ? "dark" : "light");
  const sync = () => {
    button.setAttribute("aria-label", current() === "dark" ? "Passer en clair" : "Passer en sombre");
  };

  button.addEventListener("click", () => {
    const next = current() === "dark" ? "light" : "dark";
    root.dataset.theme = next;
    try {
      localStorage.setItem("theme", next);
    } catch (e) {}
    sync();
  });
  sync();
})();

// The header gets a background once the page scrolls.
(function () {
  const top = document.getElementById("top");
  const onScroll = () => top.classList.toggle("scrolled", window.scrollY > 8);
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();
})();

// Beta sign-up: sends to data-endpoint (JSON POST) or opens a mail to data-mailto.
(function () {
  const form = document.getElementById("beta-form");
  const note = document.getElementById("beta-note");
  const say = (text, kind) => {
    note.textContent = text;
    note.className = "form-note" + (kind ? " " + kind : "");
  };

  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const email = form.email.value.trim();
    const platform = form.platform.value;
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      say("Cette adresse ne ressemble pas à une adresse e-mail.", "err");
      form.email.focus();
      return;
    }

    const endpoint = form.dataset.endpoint;
    const mailto = form.dataset.mailto;

    if (endpoint) {
      const button = form.querySelector("button");
      button.disabled = true;
      say("Envoi…");
      try {
        const res = await fetch(endpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ email, platform, source: "maya-studio.postmelo.ch" }),
          signal: AbortSignal.timeout(8000),
        });
        if (!res.ok) throw new Error(String(res.status));
        form.reset();
        say("C'est noté. Tu recevras un message dès que la bêta ouvre.", "ok");
      } catch (err) {
        say("L'inscription n'est pas passée. Réessaie dans un moment.", "err");
      } finally {
        button.disabled = false;
      }
      return;
    }

    if (mailto) {
      const subject = encodeURIComponent("Bêta Maya Studio");
      const body = encodeURIComponent(`Je veux tester Maya Studio.\n\nAdresse : ${email}\nPlateforme : ${platform}`);
      window.location.href = `mailto:${mailto}?subject=${subject}&body=${body}`;
      say("Ton logiciel de messagerie s'ouvre avec le message prêt à partir.", "ok");
      return;
    }

    say("Les inscriptions ouvrent très bientôt. Repasse dans quelques jours.");
  });
})();
