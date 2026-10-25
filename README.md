# Preuves #294, ronde 10

Tête produit `e84053c9eb08aff8997724ad856c6535fe88f609`, PR #335.

[La matrice](classe/r10-matrice.md) couvre les 32 sites Sol, 24 groupes de tests et huit précisions Astra de la [liste canonique](classe/r10-brouillon.codex.md).

[Tests Node/UI](classe/r10-node-integrated.log). 1 165 réussis, aucun échec, skip, todo ou annulation. [Tests Rust](classe/r10-rust-integrated.log). 454 réussis, aucun échec, ignoré ou filtré. [Build web](classe/r10-web-build.log), [compilation Cocoa](classe/r10-native-build.log), [format](classe/r10-fmt-integrated.log) et [gitleaks](classe/r10-gitleaks-final.log) terminés avec sortie 0.

[Parcours intégral terminé avec sortie 0](r10-final/drive-full.log). Le rapport owner porte PASS sur la tête exacte.

[Parcours navigateur et limites des fixtures](r10-final/reproduction.md), [cas observés](r10-final/cases.json), [captures](r10-final/spheres/pages), [rapport verify-ezerbot](r10-final/report.md), [Cleanup](r10-final/cleanup.json).

[Pièces jointes](r10-attachments/matrix.md) et [voix](r10-voice/matrice.md) portent leurs rouges puis verts. Le [replay shell sur les sources de départ](classe/r10-shell-baseline-confirmed-red.log) distingue les défauts comportementaux des APIs encore absentes; [115 tests ciblés verts](classe/r10-shell-confirmed-green.log) suivent ce replay.

Le retrait legacy tardif est reproduit dans [la route réelle](r10-attachments/delegation-legacy-red.log), puis corrigé dans [le passage final](r10-attachments/rust-green-complete.log). La compatibilité des anciens clients qui omettent les champs optionnels reste testée.

Le principe principle-fix-root-causes a conduit à conserver l’appartenance du brouillon et l’identité de chaque sélection. principle-prove-it-works impose les parcours Chrome sur cette tête. Ce dossier ne contient aucun verdict officiel. Le coordinateur conserve seul verifier et pret pour la ronde 10/10.
