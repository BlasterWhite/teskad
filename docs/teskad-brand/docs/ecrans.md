# Teskad — Spécification des écrans

Chaque écran a trois références :
- `maquettes/ecrans/<nom>.png` : le rendu attendu (à 2×) ;
- `maquettes/ecrans/html/<nom>.html` : la source HTML/CSS de la maquette, qui donne les valeurs exactes. Elle s'ouvre dans un navigateur, avec les polices du pack ;
- `maquettes/avant/<nom>.png` : la capture de l'interface actuelle, pour comparer.

Les maquettes font 1440 × 900. La disposition doit rester fluide : les largeurs fixes indiquées sont celles des colonnes latérales, et le reste s'étire.

**Règle de base : aucune fonction n'est ajoutée ni retirée.** Chaque élément de l'interface actuelle se retrouve dans la nouvelle, parfois déplacé (dans un sous-menu, par exemple). Les textes de l'interface restent ceux de l'app (anglais / français via i18n).

---

## 1. Workspace · vue Terminals — `01-workspace-terminals`

### Barre latérale (navigateur), 248 px, `bg`, bordure droite `border`

1. **En-tête (44 px)** : symbole Fourche 18 px (version `-small`) + « teskad » en Space Grotesk 16/600 + version en mono 11 `textMuted` ; à droite, le bouton pour masquer le navigateur (Ctrl+Alt+N).
2. **« Open the project manager »** : bouton pleine largeur, 32 px, `surface`, bordure `border`, rayon 8.
3. **Titre de section « WORKSPACES »** (label mono 11, capitales) + bouton « nouveau dossier ».
4. **Rangée de workspace (32 px, rayon 8)**, de gauche à droite :
   - indicateur d'état du workspace : cercle vide 8 px (`inactive`) au repos, arc « en cours » quand un agent travaille ;
   - nom (ellipse si trop long) ;
   - **badge de compte Claude** s'il y en a un : éclair + nom du compte (« Main »), 18 px, rayon 4, fond `surfaceRaised`, texte `status.active`. C'est le compte Claude Code, à ne pas confondre avec une branche ;
   - icône Orchestration (symbole Fourche 14 px, monochrome `inactive`). Pour le workspace actif, le symbole est en couleur, suivi du nombre d'agents en cours.
   - Workspace actif : fond `selectionBg`, bordure 1 px plus claire, texte `text` 500.
5. **Carte « Plan »** en bas (rayon 12, `surface`) : pour chaque compte, nom + heure de réinitialisation ; jauges 5 h et 7 j de 4 px avec pourcentage en mono ; « No figures yet » si pas de données.
6. **Pied (48 px)** : « + » nouveau workspace, champ Filter (30 px, `surfaceSunken`), bouton Réglages.

### Barre du haut, 44 px

- Nom du workspace en Space Grotesk 15/600.
- **Onglets** Terminals / Board / Memory / Statistics : contrôle segmenté (fond `surface`, onglet actif `border` + texte `text`).
- À droite : sélecteur de compte (éclair + « Main » + chevron, bouton à bordure), séparateur, puis disposition (avec chevron), grille, nouveau panneau, rafraîchir, plus, séparateur, inspecteur (Ctrl+Alt+I), fermer la fenêtre. Boutons icône de 28 px.

### Panneau d'agent (rayon 12, bordure `border`, corps `surfaceSunken`)

**En-tête sur une seule ligne de 40 px** (`surface`, bordure basse), de gauche à droite :

| Élément | Détail |
|---|---|
| Icône d'état | 12 px, forme + couleur d'état (voir charte §4). L'état en toutes lettres est dans « Details » et en infobulle |
| Nom de l'agent | Inter 13/600 |
| Badge de rôle | « Orchestrator » : pilule `selectionBg`, texte lavande, symbole Fourche 11 px. « Worker » : pilule à bordure `borderStrong` |
| Tâche (workers) | texte secondaire, prend la place restante, ellipse + infobulle |
| Branche | icône branche + nom en mono 11, ellipse (max ≈ 110–120 px) + infobulle |
| Contexte | mini-jauge 28 × 4 px + pourcentage mono 11 |
| **Details ▾** | bouton 22 px qui ouvre le sous-menu de session |
| Actions | 5 boutons icône 26 px : mode auto (éclair), [action % barré à libeller], agrandir, plus, fermer |

**Sous-menu « Details »** (popover `surfaceRaised`, rayon 8, ombre popover, 264 px) : il contient uniquement ce qui n'est pas dans l'en-tête :
- Agent CLI (sélecteur, ex. Claude Code ▾) ;
- Shell (bash) ;
- Session (ID en mono + bouton copier) ;
- State (icône + libellé).

**Panneau focalisé** : bordure `accent` + halo 3 px `accent` à 16 %.

**Corps** : terminal en JetBrains Mono 12,5/19, padding 14 × 16, couleurs du thème terminal. Le séparateur « ─── Mia (hangar-ai) » et la ligne de saisie restent rendus par le CLI lui-même.

### Barre d'envoi groupé, 52 px

Champ « Send the same instruction to all N panes… » (34 px, `surface`, icône diffusion, raccourci affiché en touches `Ctrl` `Shift` `B`), bouton secondaire « When free » (icône horloge), bouton principal **Broadcast** (`accent`, texte blanc 600), séparateur, branche courante et nombre de panneaux en mono 11.

---

## 2. Réglages · General — `02-settings-general`

Fenêtre modale 1040 × 840 max, rayon 12, `surface`, bordure `borderStrong`, ombre modal, sur un voile `scrim`.

- **Colonne de navigation (240 px, fond un cran plus sombre)** :
  - champ « Search a setting… » en tête ;
  - rubriques regroupées : *Application* (General, Appearance, Shortcuts), *Terminal* (Terminal, Shell), *Agents* (Accounts, Kits), *Connections* (Notifications, Mobile, Voice, Discord) ;
  - en bas, séparés : Experimental, About.
  - Toutes les rubriques actuelles sont présentes, seul le regroupement est nouveau. Rubrique active : style sélection.
- **En-tête (56 px)** : titre de la rubrique en Space Grotesk 18/600 + bouton fermer.
- **Contenu** : sections avec un petit titre (12/600, `textMuted`) puis un **bloc** (rayon 12, `surface` un cran plus clair, bordure `border`). Chaque réglage est une rangée : libellé 13/500 + description 12 `textMuted` à gauche, contrôle à droite. Les rangées sont séparées par des filets `border`.
- **Contrôles** :
  - interrupteur 36 × 20 (actif : `accent`, pastille blanche) ;
  - contrôle segmenté (System / English / Français) ;
  - champ 32 px `surfaceSunken` en mono pour les chemins + bouton « Browse… » ;
  - curseur (piste 4 px, poignée blanche 14 px avec halo accent) + valeur en mono (« 700 ms ») ;
  - badge « BETA » : mono 10, bordure, texte lavande.

Les autres rubriques reprennent exactement ce modèle de blocs et de rangées.

---

## 3. Menu du workspace + Orchestration — `03-workspace-menu-orchestration`

### Menu contextuel du workspace (248 px, `surfaceRaised`, rayon 8, ombre popover)

Toutes les entrées actuelles, dans le même ordre, avec des titres de groupe en 11 px `textMuted` et des séparateurs :
- *Go to* : agents du workspace (icône d'état + nom + rôle en mono) ;
- Rename, Open with VS Code, Git worktrees, MCP servers…, Kits…, **Orchestration…** (avec le symbole Fourche) ;
- *Claude Code account* : choix exclusif, avec une coche sur le compte actif ;
- *Move to folder* : New folder ;
- Put to sleep (désactivé quand ce n'est pas possible), **Delete workspace…** en couleur « échec ».

Rangées de 30 px, rayon 6, icônes 14 px.

### Fenêtre Orchestration (≤ 780 px, rayon 12)

- **En-tête** : symbole Fourche 36 px, titre « Orchestration » (Space Grotesk 18), sous-titre workspace · chemin en mono 12 `textMuted`, bouton fermer.
- **Models** : un bloc avec une rangée par rôle (Orchestrator, Workers, Reviewer). Chaque rangée contient :
  - une pastille d'identité 28 px (dégradé pour Orchestrator, Fourche pour Workers, œil vert pour Reviewer) ;
  - le nom du rôle et « default: … » en mono ;
  - un **contrôle segmenté opus / sonnet / haiku** (le choix actif en `accent`) ;
  - un champ libre en mono pour un identifiant complet. Le segmenté remplit le champ, et un identifiant tapé désélectionne le segmenté.
- **Codex models (BETA)** : section repliable, trois champs (Orchestrator, Workers, Reviewer) avec le texte indicatif « Codex's own default ».
- **Pied** (fond un cran plus sombre) : rappel « Applies to the next worker started and the next delivery. », puis « Restore defaults » (bouton texte), « Cancel » (secondaire) et « Save » (principal, désactivé tant que rien n'a changé).
- La partie de la fenêtre au-dessus de « Models » n'était pas visible dans la capture : elle garde son contenu actuel, mis en forme avec le même modèle de blocs.

---

## Écrans non maquettés

Board, Memory, Statistics, Réglages › Appearance (voir `themes.md`), l'inspecteur et le gestionnaire de projets n'ont pas encore de maquette. Ils doivent reprendre les mêmes composants : blocs et rangées de réglages, contrôle segmenté, badges, indicateurs d'état, menus, fenêtres modales, boutons. Leur structure fonctionnelle reste inchangée.
