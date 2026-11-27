# Teskad — Charte graphique

Version 1. Références visuelles : `maquettes/charte/logo-fourche.png` et `maquettes/charte/palette-etats-typo.png`.

## 1. Personnalité

Teskad orchestre plusieurs agents de code qui travaillent en parallèle. L'identité doit communiquer **orchestration, parallélisme, coordination, fiabilité, sophistication technique**.

- Un outil de développeur sérieux et soigné, pas un chatbot IA.
- Sombre par défaut, lisible pendant des heures, dense quand c'est utile.
- Pas de clichés IA : ni visage de robot, ni étincelles, ni halos lumineux, ni dégradés en fond.
- La couleur sert à **signifier** (action, focus, état), pas à décorer.

## 2. Le logo « Fourche »

Un nœud orchestrateur (à gauche) distribue le travail à trois agents, chacun sur sa branche. Il se lit de gauche à droite, comme le flux réel de l'application.

### Construction (grille 64 × 64)

| Élément | Valeur |
|---|---|
| Nœud orchestrateur | cercle r 9, centre (13, 32), dégradé violet → magenta |
| Agents | cercles r 5, centres (54, 14), (54, 32), (54, 50) |
| Branches | trait 4,5, bouts ronds ; courbes de Bézier qui basculent à x ≈ 34 |
| Couleurs des branches | haut lavande `#C9B6FF` (prune `#3A1F4D` sur fond clair), milieu violet `#8A4DFF`, bas magenta `#E040B0` |
| Version petite taille (≤ 24 px) | trait 7, agents r 6,5, nœud r 11 (fichiers `-small.svg`) |

### Versions

| Fichier | Usage |
|---|---|
| `logo/mark/teskad-mark-color-on-dark.svg` | symbole par défaut, sur fond sombre |
| `logo/mark/teskad-mark-color-on-light.svg` | sur fond clair |
| `logo/mark/teskad-mark-mono-white.svg` / `-mono-ink.svg` | une seule couleur : gravure, barre de menus système, documents en noir et blanc |
| `logo/lockup/teskad-lockup-*.svg` | symbole + nom, horizontal (barre de titre, site, README) |
| `logo/lockup/teskad-lockup-stacked-*.svg` | symbole au-dessus du nom (écran d'accueil, « À propos ») |
| `logo/wordmark/teskad-wordmark-on-*.svg` | nom seul, vectorisé |

Le nom s'écrit **teskad** en minuscules dans le logo, et « Teskad » dans le texte courant.

### Règles

- **Zone de protection** : un rayon de nœud (9/64 de la hauteur du symbole) tout autour.
- **Taille minimale** : 16 px pour le symbole (version `-small`), 80 px de large pour le logo horizontal.
- **Interdits** : contour, ombre portée, lueur, rotation, déformation, autres couleurs que les 4 versions, dégradé sur les branches, symbole posé sur une photo ou un dégradé chargé.
- Le logo, l'icône d'application et le dégradé de marque **ne changent pas avec le thème**.

### Icône d'application

| Fichier | Usage |
|---|---|
| `icons/app-icon.svg` | icône principale : fond dégradé, symbole blanc. Dock, barre des tâches, installeur, site |
| `icons/app-icon-dark.svg` | variante sombre : fond prune, symbole en couleur. Dans l'app, la doc, le README sombre |
| `icons/app-icon-fullbleed.svg` | sans coins arrondis, pour les OS qui appliquent leur propre masque |
| `icons/icon.ico`, `icons/icon.icns`, `icons/png/*` | exports Windows, macOS, PNG 16 → 1024 (noms compatibles Tauri inclus) |

## 3. Couleurs

### Marque

| Nom | Hex | Rôle |
|---|---|---|
| Violet | `#8A4DFF` | couleur primaire : action principale, sélection, focus |
| Magenta | `#E040B0` | secondaire : logo, dégradé ; jamais pour une erreur |
| Rose | `#FF6FB0` | ponctuel : mises en évidence rares |
| Lavande | `#C9B6FF` | surlignage, jauges, anneau de focus en sombre |
| Prune | `#3A1F4D` | profondeur, logo sur fond clair |
| Dégradé signature | `#8A4DFF → #E040B0`, 135° | **uniquement** nœud du logo, icône d'app et, au plus, le bouton d'action principale d'un écran |

### Interface (thème Teskad Dark, par défaut)

| Rôle | Token | Sombre | Clair |
|---|---|---|---|
| Fond de fenêtre | `bg` | `#0E0C12` | `#FAF8FC` |
| Surface (en-têtes, cartes) | `surface` | `#17121E` | `#FFFFFF` |
| Surface élevée (menus, popovers) | `surfaceRaised` | `#1F1828` | `#F3EFF8` |
| Surface creusée (terminal, champs) | `surfaceSunken` | `#0B0A0E` | `#F1EDF6` |
| Bordure / bordure forte | `border` / `borderStrong` | `#2A2135` / `#3A2F47` | `#E6E0EE` / `#D2C9DE` |
| Texte | `text` | `#F2EEF7` | `#1A1422` |
| Texte secondaire | `textSecondary` | `#C8BFD4` | `#4A3F58` |
| Texte discret | `textMuted` | `#A79DB5` | `#6E6380` |
| Accent (fond de bouton) | `accent` | `#8A4DFF`, texte blanc | `#8A4DFF`, texte blanc |
| Accent en texte (liens) | `accentText` | `#A97BFF` | `#5B2FC9` |
| Anneau de focus | `focusRing` | `#C9B6FF` | `#8A4DFF` |
| Sélection | `selectionBg` + `selectionBorder` | `#241A33` + `#8A4DFF` | `#EFE7FF` + `#8A4DFF` |

Toutes les valeurs sont dans `tokens/tokens.json` (source de vérité), `tokens/tokens.css` et `tokens/theme.rs`.

## 4. États

Chaque état a **une couleur et une forme**. On ne dépend jamais de la couleur seule.

| État | Forme (`icons/status/`) | Sombre | Clair |
|---|---|---|---|
| En cours | arc qui tourne (1,4 s par tour, linéaire ; fixe si mouvement réduit) | `#A97BFF` | `#6D3BE0` |
| Terminé | disque plein + coche | `#4FD8A4` | `#0F7A55` |
| Échec | disque plein + croix | `#FF6A55` | `#C9382A` |
| Actif | point plein | `#C9B6FF` | `#7A4FE0` |
| Inactif / au repos | cercle vide, libellé en texte discret | `#7D7290` | `#8E84A0` |
| Sélectionné | fond `selectionBg` + bordure 1 px `selectionBorder` | | |
| Panneau focalisé | bordure 1 px `accent` + halo 3 px `accent` à 16 % | | |
| Désactivé | texte `inactive`, pas de survol | | |
| Destructif (supprimer…) | texte et icône couleur « échec » | | |

L'échec est un **corail**, volontairement distinct du magenta de marque.

## 5. Typographie

| Police | Usage | Fichier |
|---|---|---|
| **Space Grotesk** 600 | titres de vue, de panneau, de fenêtre ; wordmark. Interlettrage −1 à −2 % | `fonts/SpaceGrotesk-VariableFont_wght.ttf` |
| **Inter** 400 / 500 / 600 | toute l'interface | `fonts/Inter-VariableFont_opsz,wght.ttf` |
| **JetBrains Mono** 400 / 500 | terminaux, branches, hashes, chemins, ID de modèle, raccourcis, labels techniques | `fonts/JetBrainsMono-VariableFont_wght.ttf` |

Échelle (taille / interligne) : label mono 11/14 en capitales +6 %, caption 12/16, small 13/18, **body 14/20** (par défaut), subtitle 16/22, title 20/26, headline 28/34, code 13/20. Pas de texte d'interface sous 11 px. Licence SIL OFL incluse : les polices peuvent être livrées avec l'app.

## 6. Espacement, formes, profondeur

- **Grille de 4 px** : 4, 8, 12, 16, 20, 24, 32, 40, 48.
- **Arrondis** : 4 px (badges, touches clavier), 6 px (boutons, éléments de menu, onglets), 8 px (champs, cartes, menus), 12 px (panneaux, fenêtres modales, blocs de réglages). Pilule (999 px) uniquement pour les badges de rôle.
- **Bordures** : 1 px partout ; 2 px pour l'anneau de focus, décalé de 2 px.
- **Ombres** : seulement pour ce qui flotte (menus, popovers, modales), voir `tokens.json › shadow`. Jamais sur les panneaux posés.
- **Hauteurs** : contrôles 28 / 32 / 40 px, rangées de liste 30 à 32 px, en-tête de panneau 40 px, barre de titre 44 px, barre d'envoi groupé 52 px, barre latérale 248 px.

## 7. Iconographie

- Trait de 1,5 px sur grille 16 px, bouts et angles arrondis, `currentColor`.
- Taille par défaut 16 px (14 px dans les rangées denses).
- Les icônes seules ont toujours un libellé accessible et une infobulle avec le raccourci quand il existe.
- Le **symbole Fourche** sert aussi d'icône fonctionnelle pour tout ce qui touche à l'orchestration (rôle Orchestrator, menu « Orchestration… », indicateur dans la liste des workspaces). En petite taille, utiliser la version `-small`.

## 8. Ton des textes

- Court, factuel, à l'infinitif ou à l'impératif pour les actions (« Broadcast », « Restore defaults »).
- Les descriptions de réglage expliquent l'effet en une phrase.
- Pas d'emoji dans l'interface.
