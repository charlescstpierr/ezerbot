# Design autour de la mascotte

L’utilisateur a demandé de refaire le wrapper en s’inspirant surtout de la mascotte animée de Grokbot. L’interface utilise des panneaux graphite, sombres par défaut, un thème clair neutre, une typographie sans sérif et un galet sombre aux yeux colorés au centre de l’accueil. Les yeux, les joues et le sourire prennent la couleur d’accent. La couleur d’accent se choisit dans les paramètres parmi neuf teintes (jaune citron par défaut); le serveur la conserve avec le thème dans `wrapper_preferences`. La même silhouette apparaît dans la marque, le composeur, les réponses et la nouvelle icône Mac.

## Référence conservée

L’[icône originale](../artifacts/client/dist/renderer/assets/app-icon-C7NKj2u7.png), inspectée visuellement, présente un corps argenté et deux yeux noirs inclinés. Le [GrokBotMark procédural](../artifacts/formatted/dist/renderer/assets/index.eager-app-BfVhIMYf.js:23940) utilise un SVG animé, avec corps et yeux distincts. Son moteur fournit une [boucle Idle_B de quatre secondes](../artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:19147), avec de petits mouvements et des clignements, ainsi qu’une [table d’expressions et d’activités](../artifacts/formatted/dist/renderer/assets/index.eager-platform-DUN5VkaE.js:20921).

La reconstruction reprend la silhouette, les yeux en capsules et ce rythme de repos. Le volume nacré vient de l’icône statique; l’original animé utilise surtout une couleur pleine. Les joues et le sourire sont des ajouts de ce design indépendant. Aucun fichier original n’a été modifié et aucune exécution authentifiée du client d’origine n’est revendiquée.

## Animation et états

[mascot.js](../app/ui/mascot.js) clone le SVG du [template HTML](../app/ui/index.html), avec des identifiants de gradients uniques. [mascot.css](../app/ui/mascot.css) anime le repos, la respiration, les clignements, la curiosité et le sourire. Un seul contrôleur suit le pointeur avec un mouvement limité et lissé. Les avatars des messages restent statiques; les mascottes de l’accueil, du composeur et du panneau latéral restent montées entre les rendus.

Le contrôleur suit les états réels du wrapper : chargement, repos, saisie, envoi ou vérification, attente de confirmation, réussite et déconnexion. La réussite vient uniquement d’une acceptation ou d’un écho du message concerné. Le bouton de l’accueil déclenche un petit salut distinct de cette confirmation. Les erreurs de transport mettent désormais la bannière et la mascotte en état déconnecté; une réponse HTTP de validation ne simule pas une panne réseau.

Le mouvement réduit désactive les animations CSS et le suivi animé du pointeur. La page cachée suspend les animations. Les frames, timers et listeners sont nettoyés au départ de la page. Les cinq ressources HTML/CSS/JS sont embarquées et servies sur loopback; l’animation fonctionne sans CDN ni moteur graphique externe.

## Vérification

[wrapper-mascot-verification.json](../evidence/wrapper-mascot-verification.json) conserve les observations du vrai serveur dans Chrome : mouvement entre deux frames, gradients valides, regard, saisie, sourire, coupure et reprise. Une session lancée avec `--force-prefers-reduced-motion` confirme la préférence active et des animations `none`. Le rendu 390 × 844 garde le composeur à l’écran sans débordement horizontal. Les captures sont conservées dans `app/qa`.

La compilation Mac, sa signature et le chargement des ressources de la mascotte sont vérifiés dans [wrapper-native-verification.json](../evidence/wrapper-native-verification.json). Les [177 tests Rust](../evidence/rust-verification.json) et les [33 contrôles HTTP](../evidence/wrapper-verification.json) passent après la refonte. Les réponses restent simulées.
