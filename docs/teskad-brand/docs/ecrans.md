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
2. **« Open the project manager »** : bouton pleine largeur, 32 px, `surface`, bordure `border`, rayon 8, avec un badge **HQ** (mono 10, fond `accent`). Quand le HQ est ouvert, le bouton prend l'état sélectionné (fond `selectionBg`, bordure `accent`). Voir §8.
3. **Titre de section « WORKSPACES »** (label mono 11, capitales) + bouton « nouveau dossier ».
4. **Rangée de workspace (32 px, rayon 8)**, de gauche à droite :
   - indicateur d'état du workspace : cercle vide 8 px (`inactive`) au repos, arc « en cours » quand un agent travaille ;
   - nom (ellipse si trop long) ;
   - **badge de compte Claude** s'il y en a un : éclair + nom du compte (« Main »), 18 px, rayon 4, fond `surfaceRaised`, texte `status.active`. C'est le compte Claude Code, à ne pas confondre avec une branche ;
   - compteurs d'agents : point plein `status.active` + nombre (« n agent finished, your turn ») et arc `status.running` + nombre (agents en cours), en mono 11 ;
   - icône Orchestration (symbole Fourche 14 px, monochrome `inactive` ; en couleur pour le workspace actif).
   - Workspace actif : fond `selectionBg`, bordure 1 px plus claire, texte `text` 500.
5. **Carte « Plan »** en bas (rayon 12, `surface`) : un bloc par compte (Main, Personal…), nom + heure de réinitialisation ; jauges 5 h et 7 j de 4 px avec pourcentage en mono ; « No figures yet » si pas de données.
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

## Cadre commun aux écrans 04 à 11

- **Navigateur et barre du haut** : identiques à §1. Les outils Disposition, Grille, Nouveau panneau et Rafraîchir n'apparaissent que dans l'onglet Terminals, comme aujourd'hui.
- **Barre d'outils de l'onglet** (Board, Memory, Statistics), 48 px sous la barre du haut, bordure basse `surfaceRaised` :
  - à gauche, le compteur en Space Grotesk 15/600 (le nombre en `text`, le reste en `textMuted`), puis les filtres ou la recherche ;
  - à droite, la note MCP (icône prise + texte 12 `textMuted`) puis l'action principale.
- **Barre d'état** (onglets autres que Terminals), 32 px, mono 11 `textMuted` :
  - le dossier du workspace ;
  - la notification d'agent (« 1 agent finished, your turn » avec un point `status.active`, ou « 1 agent working » avec un arc `status.running`), cliquable : elle ouvre le panneau concerné ;
  - la branche et le nombre de panneaux.
  - Dans l'onglet Terminals, la barre d'envoi groupé (§1) la remplace, comme aujourd'hui.
- **Place qui manque** (fenêtre < 1280 px) :
  - la note MCP se réduit à son icône, avec le texte en infobulle ;
  - les filtres passent dans un bouton « Filters ▾ » avec un badge du nombre de filtres actifs ;
  - la barre d'état garde, dans cet ordre de priorité : notification, panneaux, branche, dossier (le dossier disparaît en premier).
- Les données des maquettes sont celles des captures. Les données inventées pour illustrer un élément que les captures ne montrent pas sont signalées dans chaque section, et les textes d'interface non vus dans les captures sont listés en fin de document, pour confirmation.

---

## 4. Board — `04-board`, `04-board-light`, `04b-board-task`

### Disposition et tailles

- **Barre d'outils** : « **23** tasks », ou « **8** of 23 tasks » quand un filtre est actif.
  - Sélecteurs **Label**, **Agent** et **State** (« All », « Blocked », « Not blocked ») : boutons de 28 px, rayon 6, `surface`, bordure `border`. Le libellé est en `textMuted`, la valeur en `text` 500. Un filtre actif prend la bordure `accent` et le fond `selectionBg`.
  - « × Clear » (bouton texte) apparaît dès qu'un filtre est actif.
  - À droite : « Agents read and edit this board over MCP », puis « + Add… » (bouton secondaire 30 px).
- **Colonnes** : 4 colonnes de largeur égale, écart 12, padding 12 × 16. Chaque colonne a un rayon 12, un fond `deep` (un cran sous `bg`) et une bordure `border`.
  - En-tête de 40 px : indicateur d'état (To do : cercle vide ; In progress : arc ; Review : point plein ; Done : coche), nom 13/600, compteur en mono 11, bouton « + » de 26 px.
  - Liste des cartes avec un écart de 8 et un défilement vertical propre à chaque colonne.
  - Colonne vide : bouton « + Add… » à bordure pointillée `borderStrong`, 36 px.
- **Carte de tâche** (rayon 8, `surface`, bordure `border`, padding 10 × 12, écart 8) :
  1. ID en mono 11 `textMuted` + priorité (icône barres + « High ») ;
  2. titre 13/500, 3 lignes maximum puis ellipse ;
  3. labels : puces mono 11, 20 px, rayon 4, `surfaceRaised` ;
  4. **bloquée par une dépendance** : cadenas + « Blocked by #4de9 · titre », 12 px couleur `status.failed`, 2 lignes maximum ;
  5. **bloquée par une personne** : encart `surfaceSunken` avec l'icône échec + « Blocked: raison », puis « Only a person can unblock it » (cadenas, `textMuted`), puis les boutons « Unblock » (secondaire) et « Recheck » (texte) ;
  6. pied (filet `border`, ne passe jamais à la ligne) : indicateur d'état + **nom de l'agent, en lien qui ouvre son panneau**, puis « · Claude Code » en `textMuted`, ou « Unassigned ». À droite, en mono 11 : temps écoulé (horloge), dépendances (lien), commentaires (bulle).
- **Fiche d'une tâche** (`04b`) : panneau latéral de 520 px qui s'ouvre à droite par-dessus le board, sous la barre d'outils, au-dessus de la barre d'état. Il a une bordure gauche `borderStrong` et l'ombre modale. Contenu :
  - en-tête de 48 px : ID, badge d'état « In progress », bouton « Block » (ou « Unblock »), « … » et fermer ;
  - titre en Space Grotesk 19/600 ;
  - grille de propriétés (libellés sur 136 px) : Assigned to (+ « Open pane → »), In progress since, Priority, Labels ;
  - sections Description, Dependencies (Blocks / Blocked by), Owned files (mono 12), Comments (cartes `surface`), puis un champ « Add a comment… » et « Send ». Le contenu défile, l'en-tête reste fixe.
  - La carte ouverte prend l'état sélectionné dans sa colonne.

### Tokens

`bg`, `deep`, `surface`, `surfaceRaised`, `surfaceSunken`, `border`, `borderStrong`, `text`, `textSecondary`, `textMuted`, `accent`, `accentText`, `selectionBg`, `focusRing`, `status.*`, `shadowModal`. Arrondis 4 / 6 / 8 / 12.

### États

| Élément | Survol | Focus clavier | Sélection | Désactivé |
|---|---|---|---|---|
| Carte | fond `surfaceRaised`, bordure `borderStrong`. La priorité laisse la place au bouton « Block » (ou « Unblock »). Le temps écoulé affiche l'infobulle « In progress since 12:24 » | anneau `focusRing` 2 px décalé de 2 px ; Entrée ouvre la fiche, Espace la sélectionne | fond `selectionBg`, bordure `accent` (fiche ouverte) | — |
| Filtre | `hoverOverlay` | anneau `focusRing` | actif : bordure `accent` + `selectionBg` | — |
| « Unblock » / « Recheck » | `hoverOverlay` | anneau `focusRing` | — | « Recheck » est désactivé (arc « en cours ») pendant la nouvelle vérification |

États vide, chargement, erreur, MCP non configuré : `11-states` (première ligne). Le board vide reprend le texte actuel « No task on this board yet », « Add a task » et « Connect the agents… ».

### Place qui manque

- Sous 1180 px de zone utile, les colonnes passent en défilement horizontal, avec une largeur minimale de 260 px par colonne.
- La fiche passe en plein écran du contenu sous 960 px.
- Dans une carte étroite, « · Claude Code » disparaît en premier, puis le temps écoulé passe en infobulle.

### Variante Teskad Light

`04-board-light` : même maquette avec `themes/teskad-light.json`. Seules les couleurs changent : aucune taille, aucun arrondi, aucune icône ne diffère.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| « 23 tasks » | Barre d'outils, compteur ; « n of m tasks » quand un filtre est actif |
| Label All / Agent All / State All | Mêmes sélecteurs, plus « Clear » quand un filtre est actif |
| « Agents read and edit this board over MCP » | Barre d'outils, à droite. Si le MCP n'est pas configuré, le lien « Set up the MCP connection » remplace ce texte (`11-states`) |
| « + Add… » (barre) | Barre d'outils, bouton secondaire |
| Colonnes To do 8 / In progress 1 / Review 0 / Done 14 | Mêmes colonnes et compteurs, avec un indicateur d'état par colonne |
| « + » par colonne, « + Add… » dans une colonne vide | Identiques |
| #id, priorité « High » | Première ligne de la carte |
| Titre, labels | Identiques |
| « Blocked by #… · titre » (rouge) | Ligne cadenas + texte `status.failed` |
| « Unassigned » / agent · Claude Code | Pied de carte ; le nom de l'agent est un lien qui ouvre son panneau |
| Temps (« 6 min »), dépendances (🔗 1), commentaires (💬 2) | Pied de carte, à droite, en mono |
| Barre de défilement de colonne | Défilement propre à chaque colonne |
| Barre d'état : dossier, « 1 agent finished, your turn », main, 3 panes | Barre d'état commune |
| Tâche bloquée par une personne, « only a person can unblock it », recheck, Block / Unblock (dans le brief, pas dans la capture) | Encart de blocage dans la carte, bouton « Block » au survol et dans la fiche |
| Fiche d'une tâche ouverte (dans le brief, pas dans la capture) | `04b` : panneau latéral |

Données inventées pour l'illustration : le texte de la description, les fichiers possédés et le commentaire de Mia dans `04b`.

---

## 5. Memory — `05-memory`, `05b-memory-new-entry`

### Disposition et tailles

- **Barre d'outils** : « **3** entries », champ « Search the memory… » (300 px, icône loupe), tri en contrôle segmenté **Recent / Oldest**, puis à droite « Agents read and write this memory over MCP, from every workspace » et **New entry** (bouton principal).
- **Liste** centrée, 1120 px maximum. Chaque entrée est une rangée séparée par un filet `border`, padding 14 × 16 :
  - chevron (24 px) qui déplie l'entrée ;
  - au centre : titre 14/600, extrait 13/19 `textSecondary` sur 2 lignes, puis tags (puces mono) ;
  - colonne droite de 210 px, alignée à droite, 12 `textMuted` : « Written by **Mia** » (agent) ou « Written by **you** », « Updated … », « Created … », puis le dossier du workspace d'origine en mono 11 avec ellipse et chemin complet en infobulle.
- **Entrée dépliée** : le contenu complet, rendu en Markdown (titres, gras, listes), est dans un encart `surfaceSunken` rayon 8. Les actions « Edit » (secondaire) et « Delete » (texte `status.failed`) sont sur la ligne des tags.
- **New entry** (`05b`) : fenêtre modale de 640 px.
  - En-tête « New entry ».
  - Champs Title, Content (mono 12, 180 px de haut, Markdown) et Tags (puces supprimables + saisie).
  - **Avertissement** sous le titre quand il existe déjà : encart `selectionBg`, icône alerte `accentText`, « An entry already has this title: saving updates it instead of creating a new one. »
  - Pied : « Written by you · workspace », Cancel, Save.
  - **Edit** ouvre la même fenêtre, titrée « Edit entry ».
- **Suppression** : la confirmation est un petit dialogue d'alerte ancré sur le bouton, « Delete “titre”? » avec Cancel et Delete en couleur échec (`11-states`).

### Tokens

`surface`, `surfaceSunken`, `surfaceRaised`, `border`, `borderStrong`, `text*`, `accent`, `accentText`, `selectionBg`, `hoverOverlay`, `focusRing`, `status.failed`, `scrim`, `shadowModal`.

### États

- **Rangée** : au survol, fond `hoverOverlay` et affichage d'Edit / Delete. Au focus clavier, anneau `focusRing` ; → déplie, ← replie, Entrée ouvre Edit, Suppr ouvre la confirmation.
- **Champ de la modale** : au focus, bordure `focusRing` + halo accent à 16 %.
- **Save** : désactivé tant que le titre ou le contenu est vide.
- **Planche `11-states`** : vide (« No entry yet » + New entry), aucun résultat (« No entry matches “…” » + Clear search), confirmation de suppression.

### Place qui manque

- La colonne de métadonnées passe sous le contenu, sur une ligne, sous 1100 px.
- La note MCP se réduit à son icône.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| « 3 entries » | Compteur de la barre d'outils |
| « Search the memory… » | Champ de recherche, 300 px |
| Infobulle de l'onglet « Show the memory Ctrl+Alt+K » | Conservée sur l'onglet Memory |
| « Agents read and write this memory over MCP, from every workspace » | Barre d'outils, à droite |
| « + New entry » | Bouton principal, à droite |
| Chevron + titre « Plan — … » | Identiques ; l'entrée se déplie sur place |
| Extrait (« ## Objective … ») | Extrait sur 2 lignes ; en dépliant, Markdown rendu |
| Tags (orchestration, enssat-projet-gl) | Puces mono |
| « Written by Mia », « Updated today », chemin du workspace | Colonne de droite, avec « Created … » en plus |
| Tri récentes / anciennes, Edit / Delete, confirmation, états vide et aucun résultat (dans le brief) | Contrôle segmenté, actions de rangée, `11-states` |

Données inventées pour l'illustration : la section « Status » de l'entrée dépliée et les dates « Created ».

---

## 6. Statistics — `06-statistics`

### Disposition et tailles

- **Barre d'outils** :
  - à gauche : « Statistics », la période en contrôle segmenté **7 d / 30 d / 90 d / All**, puis le sélecteur de projet « All projects ▾ » ;
  - à droite : « Up to date · 12:34 PM » (avec la coche `status.completed`), la mesure en contrôle segmenté **Tokens / Cost**, puis le bouton icône « Read the transcripts again ».
- **Corps** : padding 16, écart 12.
  1. **4 cartes de chiffres** (rayon 12, `surface`). Chacune a un libellé 12, une mini-courbe 88 × 26 (lavande, dernier point magenta), une valeur en Space Grotesk 28 et deux lignes 12 `textMuted` (comparaison avec la période précédente, puis le détail).
     - Tokens : 681M, « 98% read from the cache ».
     - API-equivalent cost : $194, « At API list prices — not what a Pro or Max plan costs ».
     - Active time : 34 h 26, « 2 h 28 per active day ».
     - Sessions : 74, « 490 prompts ».
  2. **Per day — Tokens by kind of token** : barres empilées, axe en mono 10. Légende Output / Input / Cache write / Cache read, contrôle segmenté **Day / Week**, puis le bouton « Show as a table ».
  3. **Trois colonnes** :
     - **By project** (« Pick one to filter the page »), avec nom, sessions et durée, valeur, %, jauge ;
     - **Week rhythm**, une carte de chaleur 7 × 24 à 5 niveaux avec la légende Less / More ;
     - **By model** (jauges 6 px) au-dessus de **Heaviest sessions** (rang, nom, badge du projet, modèle et date, tokens, temps actif).
- **Couleurs des séries** : Output `#E040B0`, Input `#FF6FB0`, Cache write `#8A4DFF`, Cache read `#C9B6FF` en sombre. Ces couleurs de données suivent l'accent du thème mais gardent quatre luminosités distinctes. Chaque graphique a sa vue tableau (bouton « table ») : la couleur ne porte jamais seule l'information.

### États

- **Projet** dans « By project » : au survol `hoverOverlay`. Au clic, il filtre la page : le sélecteur affiche le nom du projet et la rangée prend l'état sélectionné.
- **Barres** : au survol, infobulle avec la date et le détail des 4 séries ; au focus clavier, ← → passe d'une barre à l'autre.
- **Planche `11-states`** : relecture des transcripts en cours (« Reading the transcripts… » + progression), période vide, erreur de lecture partielle.

### Place qui manque

- Les cartes passent sur 2 × 2 sous 1100 px.
- Les trois colonnes passent sur 1 colonne sous 960 px (dans l'ordre By project, Week rhythm, By model, Heaviest sessions).
- La légende passe sous le titre du graphique.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| « Statistics », 7 d / 30 d / 90 d / All | Barre d'outils, contrôle segmenté |
| « All projects ▾ » | Sélecteur de projet |
| « Up to date · 12:34 PM », bouton rafraîchir | Barre d'outils, à droite (le rafraîchissement relit les transcripts) |
| Tokens / Cost | Contrôle segmenté de mesure |
| 4 cartes (Tokens, API-equivalent cost, Active time, Sessions) avec mini-courbes et textes | 4 cartes, mêmes textes |
| « Per day · Tokens by kind of token », légende, bouton tableau | Graphique, légende, bouton tableau, plus le choix Day / Week |
| « By project · Pick one to filter the page » | Colonne 1 |
| « Week rhythm · Active time by day of the week and hour », Less / More, bouton tableau | Colonne 2 |
| « Heaviest sessions · In this period, by tokens » | Colonne 3, sous By model |
| Par modèle (dans le brief, pas dans la capture) | Carte « By model » |

Données inventées pour l'illustration : la répartition par modèle (Opus 5.5 / Sonnet 5.5) et le détail de la carte de chaleur.

---

## 7. Inspecteur — `07-inspector`

### Disposition et tailles

- **Panneau de droite de 300 px** (Ctrl+Alt+I), fond `bg`, bordure gauche `border`, sous la barre du haut et au-dessus de la barre d'envoi groupé. Le bouton inspecteur de la barre du haut est alors enfoncé (`selectionBg` + `accentText`).
- **En-tête** :
  - pastille 32 px avec l'initiale de l'agent et l'indicateur d'état en coin ;
  - nom en Space Grotesk 15, état (« Idle ») en `textMuted` ;
  - version du CLI (« Claude Code 2.1.332 ») ;
  - raccourci `Ctrl` `1` qui donne le focus à ce panneau ;
  - fermer.
- **Sections repliables** (en-tête 34 px, chevron, 12/600) :
  - **Session** :
    - Resumes (ID mono + « Change… »), Started, Model, MCP (+ « Change… »), Folder ;
    - bouton « New session ».
  - **Terminal** : repliée par défaut ; le shell est affiché à droite de l'en-tête.
  - **Context and cost** :
    - anneau 56 px avec le pourcentage, « 75% of context used », « 748k of 1M tokens », « The agent compacts on its own as it nears the limit. » ;
    - deux tuiles : « Session cost $29.77 in 1 h 08 » et « Claude plan 5 h / 7 d » (jauges).
  - **Board task** : la tâche prise par ce panneau, ou « No task taken by this pane. »
  - **Prompt queue** : les prompts en attente, ou « No prompt waiting. »
  - **Changed files** : les fichiers modifiés, ou « Nothing changed in this folder. »
  - **Recent activity** : heure en mono, indicateur d'état, texte sur 2 lignes.
- **Panneaux à côté de l'inspecteur** : ils passent en en-tête **étroit** (voir ci-dessous).

### États

- **Section** : au survol de l'en-tête `hoverOverlay`, au focus anneau ; Espace replie ou déplie. L'état replié est mémorisé par section.
- **« Change… »** : lien `accentText`, soulignement au survol.
- **Panneau focalisé** : l'inspecteur suit le panneau focalisé (bordure accent + halo).
- **« No pane in focus »** : état vide (`11-states`).

### Place qui manque

- **En-tête de panneau étroit** (largeur < 480 px) :
  - la branche passe dans Details ;
  - la jauge de contexte se réduit au pourcentage ;
  - mode auto, action %, agrandir et plus sont regroupés dans le bouton « … », seul « Fermer » reste.
  - La tâche (workers) reste visible, en ellipse.
- **Inspecteur** : en dessous de 1180 px de fenêtre, il passe en surimpression (avec ombre) au lieu de réduire les panneaux.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| En-tête : « M », Mia, Idle, Claude Code 2.1.332, `Ctrl` `1`, × | En-tête de l'inspecteur |
| Session : Resumes … Change…, Started, Model, MCP … Change…, Folder | Section Session, avec « New session » en plus (brief) |
| Context and cost : anneau 75 %, textes, Session cost, Claude plan 5 h / 7 j | Section Context and cost |
| Board task / Prompt queue / Changed files et leurs textes vides | Sections du même nom |
| Recent activity (heure, point, texte) | Section Recent activity, avec les indicateurs d'état du pack |
| Section Terminal (brief) | Section Terminal, repliée |
| En-têtes des panneaux Mia, Theo, Luna (statut, branche, rôle, tâche, CLI, contexte, actions) | En-tête étroit ; les éléments retirés sont dans Details ou « … » |
| Barre d'envoi groupé, main, 3 panes | Identique (§1) |
| « Jump to bottom (ctrl+End) » dans le terminal | Rendu par le terminal, inchangé |

---

## 8. HQ, gestionnaire de projets — `08-hq`

### Disposition et tailles

- **Navigateur** : le bouton « Open the project manager » porte le badge **HQ**. Dans le HQ, il prend l'état sélectionné (fond `selectionBg`, bordure `accent`) et aucun workspace n'est sélectionné. Le HQ ne figure pas dans la liste des workspaces : il les suit tous.
- **Barre du haut** :
  - le titre est « HQ » ;
  - le compte affiché est celui du HQ (« Personal ») ;
  - mêmes onglets (Terminals / Board / Memory / Statistics) ;
  - mêmes outils que dans Terminals.
- **Zone principale** :
  - le panneau de l'agent **Ava**, avec le badge de rôle « Project manager » (même style que Orchestrator) et un en-tête étroit ;
  - le **panneau Board** latéral (330 px, rayon 12, fond `deep`), avec l'en-tête « Board · 0 active tasks », les boutons filtre, ouvrir l'onglet Board et fermer, puis l'état vide « No task on this board yet », « Add a task », « Connect the agents… » ;
  - l'**inspecteur** d'Ava.
- **Barre d'envoi groupé** : « Send the same instruction to all 1 panes… », 1 pane.

### États

- Le bouton HQ suit les mêmes états que les rangées de workspace : survol, focus, sélectionné.
- Les messages « [Teskad] Event — not typed by the person. » sont rendus par le terminal : le pack n'en change pas l'affichage dans la vue Terminals. Dans la vue Chat, ils prennent la forme de §9.

### Place qui manque

- Le panneau Board se replie en premier ; il reste accessible par l'onglet Board.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| « Open the project manager » (surligné) | Même bouton, avec le badge HQ, sélectionné |
| Titre « HQ », onglets, compte « Personal », outils | Barre du haut |
| Panneau Ava (Project manager, Claude Code, bash, e25fc1…, 3 %, Idle, actions) | En-tête étroit + Details |
| Panneau « Board · 0 active tasks » (filtre, agrandir, fermer, état vide, Add a task, Connect the agents…) | Panneau Board latéral |
| Inspecteur d'Ava (Session, Context and cost, Board task, Prompt queue, Changed files « No file changed by this pane. », Recent activity) | Inspecteur (§7) |
| Barre d'envoi groupé « all 1 panes », « 1 pane » | Identique |

---

## 9. Chat view — `09-chat-view`

### Disposition et tailles

- **Barre du haut** : un second contrôle segmenté **Chat / Panes** à droite des onglets, avec Chat actif.
- **Sous-en-tête** (44 px) :
  - symbole Fourche (rôle orchestrateur), nom de l'agent en Space Grotesk 15, badge d'état (« Idle ») ;
  - à droite, en mono 11 : « Opus 5.5 · 75% context ».
- **Conversation** : colonne centrée de 760 px maximum, ancrée en bas (les derniers messages sont visibles), écart 16.
  - **Message de la personne** : bulle à droite, `surfaceRaised`, rayon 12, 14/21.
  - **Réponse de l'agent** : texte 14/22 `textSecondary`, avec du Markdown rendu (gras en `text`, listes).
  - **Appels d'outils** : bloc repliable (rayon 8, `surface`) avec « Ran N shell commands » et l'indicateur d'état. Déplié, il montre les commandes en mono et leur sortie en `textMuted`.
  - **Image** : vignette 96 × 60 + nom de fichier + dimensions en mono ; un clic l'agrandit.
  - **Message de Teskad** : encart `selectionBg` avec le symbole Fourche, « [Teskad] » en mono `accentText`, « Event — not typed by the person », l'heure, puis le texte. On distingue toujours un message de Teskad d'un message de la personne.
  - **Demande de permission** : encart à bordure `accent` + halo, avec l'icône bouclier, « Mia wants to run a command », la commande en mono et trois boutons : action principale, secondaire, texte.
- **File d'attente** : au-dessus du champ, cadre pointillé « QUEUE · n », avec les messages en attente (modifier ou retirer).
- **Champ de saisie** (rayon 12, `surface`, bordure `borderStrong`) :
  - zone de texte « Message Mia… » ;
  - sélecteur de modèle « Opus 5.5 ▾ », « Effort ▾ », bouton « Plan » ;
  - aide « Enter to send, Shift+Enter for a new line » ;
  - bouton d'envoi rond `accent`.
- **Panneau Tasks** (300 px, à droite) :
  - titre ;
  - jauge 61 % + « Done 14/23 » ;
  - groupes « IN PROGRESS 1 » et « TO DO 8 » : chaque tâche a un chevron, un titre, « agent · état », et la raison du blocage en `status.failed` s'il y en a une. La tâche en cours est surlignée.
- **Barre d'état** : « 1 agent working » (arc `status.running`).

### États

- **Demande de permission** : elle prend le focus à son arrivée ; Entrée valide l'action principale, Échap ferme sans répondre.
- **Bloc d'outils** : son en-tête est un bouton (Espace) ; état de chaque appel par indicateur.
- **Envoi** : le bouton est désactivé quand le champ est vide. Quand l'agent travaille, Entrée ajoute le message à la file (« Queue »).
- **Chargement** de la conversation : squelettes (`11-states`).

### Place qui manque

- Le panneau Tasks se replie en icône (avec la jauge en badge) sous 1180 px.
- La colonne de conversation garde au moins 480 px.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| Chat / Panes (à côté des onglets) | Contrôle segmenté dans la barre du haut |
| Sous-en-tête : icône, Mia, Idle ; « Opus 5.5 · 75% context » | Sous-en-tête |
| Bloc de texte / code de la conversation | Messages, Markdown, blocs de code |
| Liste numérotée, gras, `code` | Markdown rendu |
| Champ « Message Mia… », Opus 5.5 ▾, Effort ▾, Plan, « Enter to send, Shift+Enter for a new line », envoi | Champ de saisie, mêmes éléments |
| Panneau Tasks : 61 %, Done 14/23, IN PROGRESS 1, TO DO 8, tâches, « Unassigned · To do », raison de blocage en rouge | Panneau Tasks |
| Barre d'état « 1 agent working », main, 3 panes | Barre d'état commune |
| Appels d'outils, images, messages [Teskad], demandes de permission, file d'attente (dans le brief) | Blocs décrits ci-dessus |

Données inventées pour l'illustration : les commandes du bloc d'outils, la commande soumise à permission et le message en file d'attente.

---

## 10. Réglages › Appearance (proposition, à valider) — `10-settings-appearance`

### Disposition et tailles

Fenêtre Réglages identique à §2, avec la rubrique Appearance active et un badge « PROPOSAL » (à retirer après validation).

- **Interface** (bloc de rangées) :
  - Interface size (curseur, 100 %) ;
  - Show the navigator (`Ctrl` `Alt` `N` + interrupteur) ;
  - Show the inspector (`Ctrl` `Alt` `I` + interrupteur) ;
  - Use the system's title bar ;
  - Window opacity (curseur, %).
- **Theme** :
  - rangée **Mode** : trois choix **Dark / Light / System**, avec icônes lune, soleil et écran ;
  - rangée **Specific to this workspace** (interrupteur, « Teskad(hangar-ai) keeps its own theme ») ;
  - libellé actuel « Terminal palette — the whole window follows it », puis une grille de **vignettes à 4 colonnes**. Chaque vignette (rayon 10) montre un aperçu réel de l'interface (barre latérale, en-tête de panneau avec les indicateurs en cours / terminé / échec et l'accent, quelques lignes de terminal), calculé avec les règles de `themes.md`, puis le nom.
  - **Teskad Dark** et **Teskad Light** sont épinglés en tête (icône épingle). Le thème choisi a une bordure accent 2 px, un halo et une coche.
  - Avec « System », la grille montre deux sélections : une pour le sombre et une pour le clair.

### États

- **Vignette** : au survol, bordure `borderStrong`. Au focus, anneau. Les flèches parcourent la grille (groupe radio) ; Entrée applique le thème.
- **Curseurs** : ← → par pas de 5 %, valeur affichée en mono.

### Place qui manque

- La grille passe à 3 puis 2 colonnes, avec une vignette de 180 px minimum.

### Avant → nouveau

| Élément de la capture « avant » | Place dans la nouvelle maquette |
|---|---|
| Interface size (curseur 100 %) | Identique |
| Show the navigator · Ctrl+Alt+N | Identique, raccourci en touches |
| Show the inspector · Ctrl+Alt+I | Identique |
| Use the system's title bar | Identique |
| Window opacity (brief, hors capture) | Rangée curseur |
| Theme, « Terminal palette — the whole window follows it » | Section Theme, même libellé au-dessus de la grille |
| Specific to this workspace | Rangée interrupteur |
| Dark : Teskad Dark, Graphite, Tokyo Night, Catppuccin Mocha, Dracula, … | Grille ; Teskad Dark et Light épinglés en tête, puis les autres thèmes dans leur ordre actuel |
| Sélecteur Sombre / Clair / Système (brief) | Rangée Mode (nouveau, proposition) |

---

## 11. États vide, chargement et erreur — `11-states`

Une planche, un format unique pour chaque état :
- icône de 36 px dans une pastille `surfaceRaised`, rayon 10 ;
- titre 14/600 ;
- une phrase 12 `textMuted` ;
- au plus deux actions (une principale, une secondaire ou un lien).

Les chargements utilisent des **squelettes** à la forme du contenu, sans animation de lueur, ainsi que l'arc « en cours » avec une phrase. Les erreurs utilisent l'indicateur échec et proposent toujours une action (Retry, Read again).

| Écran | États |
|---|---|
| Board | vide (texte actuel), chargement, erreur de lecture, MCP non configuré (« Set up the MCP connection ») |
| Memory | vide, aucun résultat, confirmation de suppression |
| Statistics | relecture des transcripts (progression), période vide, erreur de lecture partielle |
| Inspecteur | « No pane in focus » |
| Chat | chargement de la conversation |

---

## Textes d'interface à confirmer

Ces textes figurent dans les maquettes mais pas dans les captures. Ils sont proposés en anglais, comme le reste de l'interface, et doivent être remplacés par les textes réels quand ils existent (ou passer par l'i18n).

| Écran | Texte proposé |
|---|---|
| Panneau, action | `[Action à confirmer]` (icône % barré) |
| 04 | « of 23 tasks », « Clear », « Unblock », « Recheck », « Only a person can unblock it », « Block », « Open pane → », « Assigned to », « In progress since », « Priority », « Labels », « Description », « Dependencies », « Blocks », « Owned files », « Comments », « Add a comment… », « Send » |
| 05 | « Recent », « Oldest », « Edit », « Delete », « Created … », « Written by you », « Title », « Content », « Tags », « Add a tag… », « An entry already has this title: saving updates it instead of creating a new one. », « Save », « Edit entry » |
| 06 | « Day », « Week », « By model », « Read the transcripts again » |
| 07 | « New session », « Terminal » (section) |
| 09 | « Queue », « Mia wants to run a command », « Allow once », « Always allow », « Deny » |
| 10 | « Mode », « Dark », « Light », « System », « Dark and light pick from their own list below; System follows your desktop. », « Window opacity », « PROPOSAL » |
| 11 | « Loading the board… », « Couldn't read the board », « Retry », « Agents cannot see this board yet », « No entry yet », « No entry matches “…” », « Clear search », « Delete “…”? », « Every workspace loses this entry. This cannot be undone. », « Reading the transcripts… », « Nothing in the last 7 days », « Couldn't read some transcripts », « Read again », « No pane in focus » (brief), « Click a pane, or press Ctrl+1 to Ctrl+9, to see what its agent is doing. », « Loading the conversation… » |

## Écrans non maquettés

Gestionnaire de projets au sens « liste des projets » (s'il existe en dehors du HQ), réglages autres que General et Appearance, menus contextuels des cartes et des entrées. Ils reprennent les mêmes composants que les écrans ci-dessus.
