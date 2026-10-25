# Découpe #294 r11

Tête produit `638abad5340b6737c16a6bc24f95f28e990cd717`, PR #335. Revert normal de 48c6e404, puis seule restitution de citation. Les fonctions retirées sont suivies dans #553 et #554.

[Diff de PR](r11-stat.txt), 25 fichiers, 1483 lignes ajoutées et 90 retirées. L’ajout après revert touche trois fichiers, avec 103 lignes ajoutées et 16 retirées.

[Commandes exactes](r11-commandes.md) et [résultats](r11-checks.json). Les [rouges](r11-red.log) montrent deux pertes de citation avant correction. Les trois protections de retrait et de citation récente sont déjà vertes. Après correction, les [53 tests ciblés](r11-green.log), les [1105 tests Node/UI](r11-node.log) et les [449 tests Rust](r11-rust.log) passent, sans échec, test sauté ou ignoré. [Build web](r11-web-build.log), bundles inchangés. [Gitleaks](r11-gitleaks.log), aucune fuite.

[Parcours réel](r11-final/drive.log) et [cas observés](r11-final/cases.json). Le refus 400 est injecté côté navigateur. La création de fil, le panneau, les citations et le réessai utilisent l’application réelle. Le fil d’origine est supprimé par l’endpoint réel. Les contrôles de plusieurs cellules, blancs, ancien accusé et renommage sont rejoués. La reprise après redémarrage est couverte par les suites Node et Rust.

[Six captures inspectées](r11-final/spheres/pages), dont les refus 400 en clair et sombre à 1140×780 et 820×560. [Doctor avant](r11-final/doctor-before.log) et [après](r11-final/doctor-after.log) confirment la tête exacte et le même binaire. [Rapport owner PASS](r11-final/report.md). Les corpus absents restent non comparés.

[Cleanup](r11-final/cleanup.json) confirme les seuls PID enregistrés et le pont pm-i294 arrêtés. Le garde conserve tous les contrôles du script partagé, qui reste intact. Aucun verdict officiel, commentaire GitHub, pret ou merge lancé par cet owner. Le principe principle-prove-it-works impose les résultats observés et les captures de cette tête.
