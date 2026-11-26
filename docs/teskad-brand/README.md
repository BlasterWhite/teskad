# Teskad — Brand pack v1

Pack d'identité visuelle à appliquer à l'application desktop Teskad (Rust). Ce fichier est écrit pour être lu par Claude Code : il décrit ce que contient le pack, les règles à respecter et la marche à suivre.

## Contexte produit

Teskad est une application desktop open source (Rust) qui orchestre plusieurs agents de code IA en parallèle : un orchestrateur planifie et découpe les tâches, chaque agent travaille dans son propre terminal et sa propre branche Git, et l'interface coordonne progression, tâches et résultats.

L'identité doit faire « outil de développeur sérieux et soigné », pas « chatbot IA ». Thème sombre en priorité, thème clair supporté.

## Contenu

```
teskad-brand/
├── README.md                ← ce fichier
├── logo/
│   ├── mark/                ← symbole seul (64×64)
│   │   ├── teskad-mark-color-on-dark.svg      (+ -small.svg pour ≤ 24 px)
│   │   ├── teskad-mark-color-on-light.svg     (+ -small.svg)
│   │   ├── teskad-mark-mono-white.svg         (+ -small.svg)
│   │   └── teskad-mark-mono-ink.svg           (+ -small.svg)
│   ├── lockup/              ← symbole + nom, horizontal et empilé, 4 variantes chacun
│   └── wordmark/            ← nom seul, vectorisé (pas besoin de la police)
├── icons/
│   ├── app-icon.svg         ← icône principale (fond dégradé, symbole blanc)
│   ├── app-icon-dark.svg    ← variante sombre (fond prune, symbole couleur)
│   ├── app-icon-fullbleed.svg ← sans coins arrondis, pour les OS qui appliquent leur masque
│   ├── icon.ico / icon.icns ← Windows / macOS
│   ├── png/                 ← 16 → 1024 px, + 32x32.png, 128x128.png, 128x128@2x.png, icon.png (noms Tauri)
│   └── status/              ← icônes d'état 16 px (currentColor)
├── tokens/
│   ├── tokens.json          ← source de vérité
│   ├── tokens.css           ← variables CSS (si l'UI est web : Tauri, Dioxus, Leptos…)
│   └── theme.rs             ← constantes Rust indépendantes du framework
└── fonts/                   ← Space Grotesk, Inter, JetBrains Mono (variables, licence OFL incluse)
```

`tokens.css` et `theme.rs` sont générés à partir de `tokens.json`. En cas de modification, modifier le JSON puis répercuter dans les deux autres fichiers.

## Le logo : « Fourche »

Un nœud orchestrateur à gauche distribue le travail à trois agents, chacun sur sa branche. On le lit de gauche à droite, comme le flux réel de l'application.

- **Construction** (grille 64) : nœud r 9 en (13, 32) ; agents r 5 en x 54, y 14 / 32 / 50 ; branches de 4,5 d'épaisseur, bouts ronds.
- **Couleurs** : branche haute lavande (#C9B6FF, ou prune #3A1F4D sur fond clair), branche médiane violet #8A4DFF, branche basse magenta #E040B0, nœud en dégradé violet → magenta.
- **Petites tailles** : en dessous de 24 px, utiliser les fichiers `-small.svg` (traits et points épaissis).
- **Zone de protection** : un rayon de nœud (9/64 de la hauteur du symbole) tout autour.
- **À ne pas faire** : pas de contour, d'ombre ni de lueur, pas de rotation, pas de recoloration hors des 4 variantes, pas de dégradé sur les branches.

## Palette

Les couleurs de marque restent rares dans l'interface. Les surfaces sont neutres, teintées de prune, et les accents sont réservés à l'action, au focus et au statut.

| Rôle | Sombre | Clair |
|---|---|---|
| Fond | `#0E0C12` | `#FAF8FC` |
| Surface | `#17121E` | `#FFFFFF` |
| Surface élevée | `#1F1828` | `#F3EFF8` |
| Surface creusée (terminal) | `#0B0A0E` | `#F1EDF6` |
| Bordure / forte | `#2A2135` / `#3A2F47` | `#E6E0EE` / `#D2C9DE` |
| Texte / secondaire / discret | `#F2EEF7` / `#C8BFD4` / `#A79DB5` | `#1A1422` / `#4A3F58` / `#6E6380` |
| Accent (boutons) | `#8A4DFF`, texte blanc | `#8A4DFF`, texte blanc |
| Accent en texte (liens) | `#A97BFF` | `#5B2FC9` |
| Sélection | fond `#241A33` + bordure `#8A4DFF` | fond `#EFE7FF` + bordure `#8A4DFF` |

**Dégradé signature** violet → magenta (135°) : uniquement pour le nœud du logo, l'icône d'application et, au plus, le bouton d'action principale d'un écran. Jamais en fond de panneau, jamais sur du texte.

## États des agents et des tâches

Chaque état a **une couleur et une forme**, pour ne jamais dépendre de la couleur seule.

| État | Icône (`icons/status/`) | Sombre | Clair |
|---|---|---|---|
| En cours | arc qui tourne (`status-running.svg`) | `#A97BFF` | `#6D3BE0` |
| Terminé | coche pleine | `#4FD8A4` | `#0F7A55` |
| Échec | croix pleine | `#FF6A55` | `#C9382A` |
| Actif | point plein | `#C9B6FF` | `#7A4FE0` |
| Inactif | cercle vide, libellé en texte discret | `#7D7290` | `#8E84A0` |
| Sélectionné | fond + bordure de sélection (voir palette) | — | — |

- L'échec est un corail, volontairement distinct du magenta de marque. Ne jamais utiliser le magenta pour signaler une erreur.
- Pour « en cours », l'arc tourne (1,4 s par tour, linéaire). Avec `prefers-reduced-motion`, l'arc reste fixe.
- Les icônes `completed` et `failed` dessinent leur symbole intérieur avec `var(--tk-bg)`. Hors web, remplacer par la couleur de fond du thème.

## Typographie

- **Space Grotesk 600** : titres de vue, en-têtes de panneau, wordmark. Interlettrage −1 à −2 %.
- **Inter 400/500/600** : toute l'interface. Corps de texte 14 px, minimum 12 px.
- **JetBrains Mono** : terminaux, noms de branches, hashes, chemins, raccourcis clavier, labels techniques.

Les fichiers sont dans `fonts/` (licence SIL OFL, redistribuable avec l'app). L'échelle typographique est dans `tokens.json › type`.

## Espacements, rayons, tailles

- Grille de 4 px (`space`).
- Rayons : 6 px pour les contrôles, 8 px pour les cartes et les champs, 12 px pour les panneaux et les modales. Jamais de « pilule » sauf pour les badges.
- Contrôles de 28 / 32 / 40 px de haut, icônes de 16 px par défaut.
- Bordures de 1 px ; anneau de focus de 2 px en `focusRing`, décalé de 2 px.

## Marche à suivre pour Claude Code

1. **Identifier la stack UI** du dépôt (egui, iced, gpui, Slint, Tauri + front web, etc.) et l'endroit où le thème est défini aujourd'hui.
2. **Centraliser le thème** : importer `theme.rs` (ou `tokens.css` si l'UI est web) comme unique source de couleurs, polices, espacements et rayons. Remplacer les couleurs codées en dur par les tokens sémantiques (`DARK.surface`, `--tk-text-muted`…), jamais par les couleurs `brand` directement.
3. **Charger les polices** depuis `fonts/` et les enregistrer dans le framework UI, avec les polices de secours définies dans `tokens.json › font.fallback`.
4. **Remplacer les assets** : icône d'application (`icons/`, noms compatibles Tauri dans `png/`), logo de la barre de titre ou de l'écran d'accueil (`logo/lockup/…-color-on-dark.svg`), favicon ou petites tailles (`-small.svg`).
5. **Unifier les états** : un seul composant « badge / pastille d'état » utilisé partout (liste des agents, tâches, terminaux), basé sur le tableau ci-dessus.
6. **Ne changer aucun comportement** : cette étape porte sur l'identité visuelle (couleurs, typo, logo, icônes, états). La refonte des layouts viendra séparément. Ne pas supprimer ni ajouter de fonctionnalités.
7. **Thème clair** : brancher le thème `LIGHT` sur le même mécanisme, sélectionnable, avec le sombre par défaut.
8. **Vérifier** : contraste du texte ≥ 4,5:1 (les tokens le respectent, ne pas les éclaircir) ; focus clavier visible sur tous les éléments interactifs.
