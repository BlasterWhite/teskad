# Teskad — Guide d'implémentation (pour Claude Code)

Objectif : appliquer la nouvelle identité et les nouvelles maquettes à l'application, **sans changer son comportement**.

## Avant de commencer

1. Lire, dans cet ordre : `docs/charte-graphique.md`, `docs/themes.md`, `docs/ecrans.md`.
2. Ouvrir les PNG de `maquettes/ecrans/` et les comparer à `maquettes/avant/`.
3. Repérer dans le dépôt :
   - la bibliothèque d'interface utilisée (egui, iced, gpui, Slint, Tauri + front web…) ;
   - l'endroit où sont définis aujourd'hui les couleurs, le thème et les palettes de terminal (les thèmes Teskad Dark, Graphite, Tokyo Night, Catppuccin Mocha, Dracula, etc.) ;
   - le calcul actuel des couleurs d'interface à partir du fond du terminal et de l'accent.
4. Proposer un plan court avant d'écrire du code. Travailler par étapes, en vérifiant que l'app compile et que les tests passent à chaque étape.

## Étapes

### 1. Moteur de thème

- Adopter le format de `themes/theme.schema.json` pour tous les thèmes. Convertir les thèmes existants : leurs couleurs de terminal vont dans `terminal`, et `ui` reste vide sauf exception.
- Remplacer le calcul actuel des couleurs d'interface par les règles de `docs/themes.md` (dérivation, puis contrôle de contraste).
- Teskad Dark et Teskad Light utilisent les fichiers fournis tels quels, sans calcul.
- Exposer **une seule** structure de couleurs sémantiques à toute l'interface (voir `tokens/theme.rs › Palette`, ou les variables `--tk-*` de `tokens/tokens.css` si l'interface est web). Aucun composant ne doit contenir de couleur codée en dur.

### 2. Fondations communes

- Constantes d'espacement, d'arrondi, de taille et d'échelle typographique depuis `tokens/` (`theme.rs` ou `tokens.css`). Elles ne dépendent pas du thème.
- Charger les trois polices de `fonts/` dans la bibliothèque d'interface, avec les polices de secours de `tokens.json › font.fallback`. Les terminaux utilisent JetBrains Mono par défaut : la taille reste réglable dans Terminal › Typography.

### 3. Assets

- Icône d'application : `icons/` (noms Tauri dans `icons/png/`, `.ico`, `.icns`).
- Logo dans l'app : en-tête du navigateur (symbole `-small` + mot « teskad » rendu dans la police), écran « À propos » (`logo/lockup/teskad-lockup-stacked-*`).
- Icônes d'état : `icons/status/*.svg`, ou leur équivalent dessiné dans la bibliothèque d'interface avec les mêmes formes.

### 4. Composants partagés

À créer une fois et à réutiliser partout :

| Composant | Référence |
|---|---|
| Bouton : principal, secondaire, texte, icône seule (26 / 28 / 32 px) | maquettes 01 et 03 |
| Contrôle segmenté (onglets, langue, modèles) | 01 barre du haut, 02, 03 |
| Interrupteur, curseur, champ texte / mono, menu déroulant | 02 |
| Rangée de réglage + bloc de réglages | 02 |
| Indicateur d'état (5 formes) | charte §4 |
| Badge de rôle (pilule), badge de compte (éclair), badge BETA | 01, 02 |
| Menu contextuel (titres de groupe, séparateurs, coche, entrée destructive) | 03 |
| Popover (sous-menu Details) | 01 |
| Fenêtre modale (en-tête, contenu, pied) | 02, 03 |
| Jauges (plan, contexte) | 01 |
| Raccourci clavier affiché (`kbd`) | 01 barre d'envoi |

États obligatoires pour chaque élément interactif : normal, survol (`hoverOverlay`), appuyé, **focus clavier visible** (`focusRing` 2 px), désactivé, sélectionné.

### 5. Écrans

Dans cet ordre, en suivant `docs/ecrans.md` :

1. navigateur ;
2. barre du haut ;
3. en-tête de panneau sur une ligne + sous-menu Details ;
4. barre d'envoi groupé ;
5. fenêtre Réglages (navigation regroupée, blocs) ;
6. menu du workspace ;
7. fenêtre Orchestration.

Puis appliquer les mêmes composants aux écrans non maquettés.

### 6. Vérifications

- [ ] Chaque fonction visible dans `maquettes/avant/` est toujours accessible (au besoin via un sous-menu).
- [ ] Aucune couleur codée en dur hors du moteur de thème.
- [ ] Teskad Dark rendu identique aux maquettes. Teskad Light cohérent.
- [ ] Changer de thème recolore toute la fenêtre, mais pas les polices, les arrondis, les espacements, les icônes ni le logo.
- [ ] Contraste du texte ≥ 4,5:1 et des icônes d'état ≥ 3:1 dans tous les thèmes fournis.
- [ ] Navigation au clavier complète, focus visible, raccourcis existants inchangés.
- [ ] `prefers-reduced-motion` (ou l'équivalent système) : l'arc « en cours » ne tourne plus.

## Ce qu'il ne faut pas faire

- Ne pas ajouter de fonctionnalité, ni en retirer.
- Ne pas copier le HTML des maquettes dans une app non web : il sert de référence de valeurs, pas de code source.
- Ne pas utiliser le dégradé de marque en dehors du logo, de l'icône d'application et, au plus, du bouton principal d'un écran.
- Ne pas utiliser le magenta pour signaler une erreur.
- Ne pas modifier les fichiers générés (`tokens.css`, `theme.rs`) à la main : modifier `tokens.json`, puis régénérer.
