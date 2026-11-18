// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { redactStreamTarget } from "../src/components/OutputsPanel";
import { reconcileSelection } from "../src/selection";
import { isInteractiveTarget } from "../src/shortcuts";

describe("redactStreamTarget", () => {
  it("retire la clé RTMP (dernier segment du chemin)", () => {
    expect(redactStreamTarget("rtmp://live.exemple.com/app/ma-cle-secrete")).toBe("rtmp://live.exemple.com/app");
    expect(redactStreamTarget("rtmps://a.rtmp.youtube.com:443/live2/abcd-efgh")).toBe("rtmps://a.rtmp.youtube.com:443/live2");
  });

  it("garde un chemin RTMP sans clé", () => {
    expect(redactStreamTarget("rtmp://127.0.0.1/live")).toBe("rtmp://127.0.0.1/live");
  });

  it("retire la requête SRT (streamid, passphrase) et les identifiants", () => {
    expect(redactStreamTarget("srt://ingest.exemple.com:9000?streamid=#!::r=live/cle,m=publish&passphrase=secret")).toBe(
      "srt://ingest.exemple.com:9000",
    );
    expect(redactStreamTarget("rtmp://user:motdepasse@serveur/app/cle")).toBe("rtmp://serveur/app");
  });

  it("n'affiche rien pour une destination absente ou illisible", () => {
    expect(redactStreamTarget(undefined)).toBe("");
    expect(redactStreamTarget("pas une url avec clé")).toBe("");
  });
});

describe("reconcileSelection", () => {
  it("garde une source présente", () => {
    expect(reconcileSelection(["a", "b"], "a", null, 0)).toBe("a");
  });

  it("garde une source ajoutée dont l'événement n'est pas encore arrivé", () => {
    expect(reconcileSelection(["a", "b"], "new", { id: "new", until: 1000 }, 500)).toBe("new");
  });

  it("revient à la plus haute après l'échéance, ou sans attente", () => {
    expect(reconcileSelection(["a", "b"], "new", { id: "new", until: 1000 }, 1500)).toBe("b");
    expect(reconcileSelection(["a", "b"], "gone", null, 0)).toBe("b");
    expect(reconcileSelection(["a", "b"], null, null, 0)).toBe("b");
    expect(reconcileSelection([], null, null, 0)).toBeNull();
  });
});

describe("isInteractiveTarget", () => {
  const el = (html: string) => {
    const div = document.createElement("div");
    div.innerHTML = html;
    document.body.appendChild(div);
    return div.firstElementChild!;
  };

  it("laisse Entrée et Espace aux boutons, liens et champs", () => {
    expect(isInteractiveTarget(el("<button>Arrêter le direct</button>"))).toBe(true);
    expect(isInteractiveTarget(el("<button><svg></svg></button>").querySelector("svg"))).toBe(true);
    expect(isInteractiveTarget(el('<a href="#">lien</a>'))).toBe(true);
    expect(isInteractiveTarget(el('<div role="radio">x</div>'))).toBe(true);
    expect(isInteractiveTarget(el('<li role="option">x</li>'))).toBe(true);
    expect(isInteractiveTarget(el("<input />"))).toBe(true);
    expect(isInteractiveTarget(el("<textarea></textarea>"))).toBe(true);
    expect(isInteractiveTarget(el("<select></select>"))).toBe(true);
  });

  it("laisse passer les raccourcis ailleurs", () => {
    expect(isInteractiveTarget(document.body)).toBe(false);
    expect(isInteractiveTarget(el("<div>panneau</div>"))).toBe(false);
    expect(isInteractiveTarget(null)).toBe(false);
  });
});
