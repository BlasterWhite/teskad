# Teskad — Thèmes

## Principe

**Un thème ne change que les couleurs :**

- l'apparence, sombre ou claire ;
- les couleurs des terminaux : fond, texte, curseur, sélection et les 16 couleurs ANSI ;
- les couleurs de l'interface : barres, panneaux, cartes, bordures, texte, accent, couleurs des états.

**Tout le reste est commun à tous les thèmes et vient du pack Teskad :**

- les polices : Inter pour l'interface, Space Grotesk pour les titres, JetBrains Mono pour les terminaux ;
- les arrondis (6, 8 et 12 px), les espacements et les tailles ;
- les icônes, les indicateurs d'état et la disposition.

Le logo, l'icône d'application et le dégradé de marque restent fixes eux aussi. Seul le texte « teskad » à côté du symbole suit la couleur de texte du thème.

Toute la fenêtre suit le thème choisi, comme aujourd'hui. Le réglage « Specific to this workspace » reste disponible.

## Format

Un thème est un fichier JSON validé par `themes/theme.schema.json` :

```json
{
  "id": "tokyo-night",
  "name": "Tokyo Night",
  "appearance": "dark",
  "terminal": {
    "background": "#1A1B26", "foreground": "#C0CAF5", "cursor": "#C0CAF5", "selection": "#33467C",
    "ansi": { "black": "…", "red": "…", "green": "…", "yellow": "…", "blue": "…", "magenta": "…", "cyan": "…", "white": "…",
              "brightBlack": "…", "brightRed": "…", "brightGreen": "…", "brightYellow": "…", "brightBlue": "…", "brightMagenta": "…", "brightCyan": "…", "brightWhite": "…" }
  },
  "ui": { }
}
```

- `terminal` est obligatoire.
- `ui` est facultatif. Chaque clé présente l'emporte, et chaque clé absente est **dérivée** de `terminal` selon les règles ci-dessous. Les clés possibles sont les mêmes que `tokens.json › theme.dark` (`bg`, `surface`, `surfaceRaised`, `surfaceSunken`, `border`, `borderStrong`, `text`, `textSecondary`, `textMuted`, `accent`, `accentHover`, `accentPressed`, `onAccent`, `accentText`, `focusRing`, `selectionBg`, `selectionBorder`, `hoverOverlay`, `scrim`, `status.{running, completed, failed, active, inactive}`).
- **Teskad Dark** (`themes/teskad-dark.json`, par défaut) et **Teskad Light** (`themes/teskad-light.json`) définissent toutes les clés `ui` : ils affichent exactement la palette de marque, sans aucun calcul.

## Dérivation des couleurs d'interface

Notations : `B` = `terminal.background`, `F` = `terminal.foreground`, `mix(a, b, p)` = mélange de `a` vers `b` à `p` % (en sRGB linéaire de préférence), `ansi.x` = couleur ANSI.

| Token | Règle |
|---|---|
| `bg` | `B` |
| `surfaceSunken` | `mix(B, black, 20 %)` en sombre, `mix(B, F, 2 %)` en clair |
| `surface` | `mix(B, F, 4 %)` |
| `surfaceRaised` | `mix(B, F, 7 %)` |
| `border` | `mix(B, F, 12 %)` |
| `borderStrong` | `mix(B, F, 18 %)` |
| `text` | `F` |
| `textSecondary` | `mix(B, F, 75 %)` |
| `textMuted` | `mix(B, F, 60 %)` |
| `accent` | `ansi.magenta` par défaut. Un thème dont la couleur principale est autre (bleu pour Tokyo Night, par exemple) la fixe avec `"ui": {"accent": "…"}` |
| `accentHover` / `accentPressed` | `mix(accent, F, 12 %)` / `mix(accent, B, 12 %)` |
| `onAccent` | blanc ou `B`, celui qui contraste le plus avec `accent` |
| `accentText` | `accent` éclairci ou foncé jusqu'à 4,5:1 sur `bg` |
| `focusRing` | `accentText` |
| `selectionBg` | `mix(B, accent, 14 %)` |
| `selectionBorder` | `accent` |
| `status.running` | `accentText` |
| `status.completed` | `ansi.green` |
| `status.failed` | `ansi.red` |
| `status.active` | `mix(accent, F, 40 %)` |
| `status.inactive` | `mix(B, F, 45 %)` |

## Contrôle de contraste (obligatoire, après dérivation)

- `text`, `textSecondary`, `textMuted` et `accentText` doivent atteindre **4,5:1** sur `bg` et sur `surface`. Sinon, on les rapproche de `F` jusqu'à atteindre le seuil.
- `onAccent` doit atteindre 4,5:1 sur `accent`.
- Les couleurs d'état doivent atteindre **3:1** sur `surface` (icônes). Sinon, on reprend la couleur d'état du pack (`tokens.json`, même apparence).
- `border` doit rester visible : au moins 1,3:1 sur `bg`.

Les **formes** des indicateurs d'état ne changent jamais : un état reste lisible même si un thème rapproche deux couleurs.

## Écran Apparence (recommandation)

1. Un sélecteur **Sombre / Clair / Système**.
2. Une grille de thèmes, avec **Teskad Dark** et **Teskad Light** épinglés en tête. Chaque vignette montre un aperçu réel de l'interface (barre latérale, en-tête de panneau, trois pastilles d'état, quelques lignes de terminal) plutôt que seulement des lignes de code.
3. Le reste de l'écran actuel (taille d'interface, navigateur, inspecteur, barre de titre système, « Specific to this workspace ») est conservé.
