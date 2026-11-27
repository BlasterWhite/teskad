# Teskad — Brand & UI pack v3

Pack d'identité visuelle et de maquettes pour la refonte de Teskad, l'application desktop (Rust) qui orchestre des agents de code IA en parallèle.

**Pour Claude Code** : commencer par `docs/implementation.md`, qui donne l'ordre de lecture, les étapes et la liste de vérifications.

## Contenu

```
teskad-brand/
├── README.md                 ← ce fichier
├── docs/
│   ├── implementation.md     ← marche à suivre pour appliquer la refonte (à lire en premier)
│   ├── charte-graphique.md   ← logo, couleurs, états, typographie, formes, icônes, ton
│   ├── themes.md             ← ce qu'un thème change, format, dérivation, contrastes
│   └── ecrans.md             ← spécification détaillée de chaque écran maquetté
├── maquettes/
│   ├── ecrans/               ← rendus PNG (2×) des nouveaux écrans
│   │   └── html/             ← sources HTML/CSS des maquettes (valeurs exactes)
│   ├── avant/                ← captures de l'interface actuelle, pour comparer
│   └── charte/               ← planches logo et palette (PNG + HTML)
├── themes/
│   ├── theme.schema.json     ← format d'un thème
│   ├── teskad-dark.json      ← thème par défaut (palette de marque exacte)
│   └── teskad-light.json
├── tokens/
│   ├── tokens.json           ← source de vérité : couleurs, typo, espacements, arrondis, tailles, mouvement
│   ├── tokens.css            ← généré (interface web)
│   └── theme.rs              ← généré (constantes Rust, indépendant de la bibliothèque d'interface)
├── logo/                     ← symbole, logo horizontal et empilé, mot seul ; 4 versions de couleur
├── icons/                    ← icône d'application (SVG, PNG 16→1024, .ico, .icns) + icônes d'état
├── fonts/                    ← Space Grotesk, Inter, JetBrains Mono (licence OFL incluse)
└── tools/gen_tokens.py       ← régénère tokens.css et theme.rs depuis tokens.json
```

## Les écrans

Chaque écran : `maquettes/ecrans/<nom>.png` (rendu 2×, Teskad Dark), `maquettes/ecrans/html/<nom>.html` (source), section dans `docs/ecrans.md` avec son tableau avant → nouveau, capture d'origine dans `maquettes/avant/`.

| Nouveau | Avant |
|---|---|
| `01-workspace-terminals` | `01-workspace-terminals.png` |
| `02-settings-general` | `02-settings-general.png` |
| `03-workspace-menu-orchestration` | `03a-workspace-menu.png`, `03b-orchestration.png` |
| `04-board`, `04-board-light` (Teskad Light), `04b-board-task` | `04-board.png` |
| `05-memory`, `05b-memory-new-entry` | `05-memory.png` |
| `06-statistics` | `06-statistics.png` |
| `07-inspector` | `07-inspector.png` |
| `08-hq` | `08-hq.png` |
| `09-chat-view` | `09-chat-view.png` |
| `10-settings-appearance` (proposition à valider) | `10-settings-appearance.png` |
| `11-states` : états vide, chargement et erreur | — |

Les fichiers HTML de `maquettes/*/html/` s'ouvrent directement dans un navigateur et utilisent les polices de `fonts/`.

## Les règles à ne jamais oublier

1. **Aucune fonction ajoutée ni retirée.** La refonte change l'apparence et la disposition, pas le comportement.
2. **Un thème ne change que les couleurs.** Les polices, arrondis, espacements, tailles, icônes, indicateurs d'état, la disposition et le logo sont communs à tous les thèmes.
3. **Chaque état a une couleur et une forme.**
4. **Le dégradé de marque est rare** : logo, icône d'application, au plus le bouton principal d'un écran.
5. **Contraste** : texte ≥ 4,5:1, icônes d'état ≥ 3:1, dans tous les thèmes.
6. **Aucune couleur codée en dur** hors du moteur de thème.

## Points ouverts

- L'action de panneau dont l'icône ressemble à un « % barré » doit garder son libellé actuel (marqué `[Action à confirmer]` dans les maquettes).
- Les textes d'interface qui n'apparaissent pas dans les captures sont listés à la fin de `docs/ecrans.md` (« Textes d'interface à confirmer »).
- L'écran 10 (Appearance) est une proposition à valider avant implémentation.
