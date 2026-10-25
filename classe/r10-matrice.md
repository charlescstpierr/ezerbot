# Matrice de correction #294, ronde 10

Tête intégrée `e84053c9eb08aff8997724ad856c6535fe88f609`. Preuve navigateur complète terminée avec sortie 0. `r10-final/drive-full.log` porte le parcours intégral, `r10-final/report.md` le PASS owner et `r10-final/cleanup.json` le nettoyage limité à pm-i294. La liste canonique `r10-brouillon.codex.md` reste intacte. Cette matrice distingue défaut reproduit, conservation, et état synthétique sans déclencheur de production établi.

Les références S désignent `app/ui/draft-transfer.test.cjs`, P `app/ui/panels/pages.test.cjs`, A `app/web/src/attachments-ui.test.js`, V `app/ui/panels/voice.test.cjs`, VC `app/web/src/voice-conversation-ui.test.js`, M `app/web/src/meetings-ui.test.js`. Les chemins de preuves sont relatifs à `evidence/i294`.

Le journal `classe/r10-shell-baseline-confirmed-red.log` exécute les tests actuels contre les sources exactes de `1f0b03c7` conservées sous `classe/baseline`. Les échecs d'API nouvelle ne sont pas comptés comme reproductions comportementales. `classe/r10-shell-confirmed-green.log` donne 115/115 après ce replay. `classe/r10-shell-red.log` porte les premiers rouges de restitution. `classe/r10-shell-races-red3.log` reproduit les trois courses découvertes pendant la correction. Le test bloqué d'une fixture sans onInput a été arrêté uniquement sur autorisation explicite, puis corrigé. Son exécution interrompue ne compte pas comme verte.

## Sites Sol

| Site | Qualification et décision | Tests et preuve |
| --- | --- | --- |
| R01 | Défaut. Le shell restitue les captures aux panneaux après le texte et le document. | S T01-T03, rouge shell, vert final. |
| R02 | Défaut. L'orpheline suit le transfert new vers le fil créé avant le rendu. | S T01/T02/T24, preuve live orpheline 400 et 409. |
| R03 | Défaut citation. Refus distingué de prise en charge serveur. Fichiers retenus. | S T01/T02/T04, A rétention. |
| R04 | Défaut citation et tentative non durable. Reprise locale persistée avec ID, body et extras exacts. | S T05/T22 avec refresh réussi et échoué, boutons React live. |
| R05 | Conservation. iPhone ne monte ni Pages ni voix. Le refus initial retire l'optimiste, l'erreur réseau avant refresh désactivé conserve la tentative. | S T06 envoi et refresh, iphone.test.cjs, A. |
| R06 | Conservation stable étendue à restitution complète. Création refusée garde saisie récente et document. | S T03/T21, V T17, A création. |
| R07 | Défaut. Recherche par conversation de soumission. Fin tardive de sous-fil ne sélectionne plus B pendant busy. | S T07 enregistré et refusé, N06. |
| R08 | Conservation. onSent uniquement sur DTO serveur indépendant, une fois, tous états durables. | S T04 queued/dispatching/unknown/accepted/echoed, V T14, A T13/T14. |
| R09 | Défaut de propriétaire. Fil absent conservé pour récupération explicite, brouillon new distinct intact. Cache mémoire par clé. | S T09/N03, T21, N06. |
| R10 | Conservation des gardes synchrones. Les fins tardives et refresh explicites sont testés. | S T07/T08/T09/N06, P navigation refusée. |
| R11 | Défauts openThread et création de bot tardive. Vérification busy au retour. navigateLink conserve son contrat synchrone. | S T07/T08. |
| R12 | Ancien accusé déjà protégé. Nouvelle restitution protégée aussi contre croix et navigation explicites. | S T12, P accusés, live citation récente même passage. |
| R13 | Conservation étendue. Restitution relit le titre courant, sans remplacer une citation plus récente. | S T10/T12, P renommage, live Forfaits vérifiés. |
| R14 | Défaut requalification. Passage absent, Page absente ou erreur annoncés au toast final. Échec réseau ne signifie pas suppression. Ordre des Pages ne change plus la citation. | S T10/T11/N02, P citabilité. |
| R15 | Conservation. Même ID, ancre et fichiers pour reprise et repli 404. Renvoyer normal reste serveur. | S reprise et repli, live boutons React, Rust pages et tests crash. |
| R16 | Conservation stricte. Aucun relâchement check_resend. Différence de version seule admise, ajout/retrait/changement d'ancre 409. | Rust tests.rs, pages/anchors.rs, tests/pages.rs. |
| R17 | Défaut de transfert et occurrence corrigé. Buckets gardés au refus, consommés par identité de sélection. | A T13-T15/T24, storage/delegation, matrice fichiers. |
| R18 | Défaut confirmé. Fin de tâche ne supprime plus texte ni fichiers du brouillon refusé. Explication visible, envoi désactivé. | S T16, V T16, delegation replay après fin de tâche. |
| R19 | Défauts mémo en sauvegarde et fil devenu absent. Transfert de session avant publication du contexte. Abandon impossible annoncé. | V T09/T14/T16/T17, matrice voix. |
| R20 | Composer.jsx non importé, hors chemin principal. Aucun correctif à ce composant inutilisé. | main.jsx mountInput, S document JSON et mentions, preuve navigateur éditeur réel. |
| R21 | Défaut structure du document lors de leaveEmptyThread. Texte et JSON transférés ensemble. | S R21/T21 changement de bot après refus. |
| R22 | Conservation du journal et défaut de fenêtre avant journal distingués. Parcours React exigé au-delà des callbacks. | S reprise durable, live menu Actions puis Renvoyer. |
| R23 | Hypothèse conditionnelle reproduite par omission synthétique du fil créé. Utilisation de la réponse de création supprime la dépendance au refresh. | V T17 rouge puis vert. |
| R24 | Défaut mémoire par fil. Échec de persistance bloque avant soumission, conserve le document et annonce le stockage indisponible. | S T21 refus avant envoi et aller-retour sans stockage. |
| R25 | Défaut UI et durable confirmé. Nouveau token de même contenu survit à ancien commit, ancien accusé et retrait tardif d’une sélection legacy sans token. Replays ne consomment pas de nouvelle sélection. | A T13/T14, Rust storage/delegation rouges directs puis verts. Retrait legacy rouge delegation-legacy-red puis vert rust-green-complete. |
| R26 | Conservation. Import/préparation échoués gardent le File. Réponses tardives respectent retrait et révision. | A T15 et tests préexistants. |
| R27 | Hypothèses reproduites. Restauration disque tardive attend les imports, respecte retraits et donne priorité à la sélection récente. Anciennes occurrences dédupliquées libérées durablement. | A T18, rouge initial et rouge retry. |
| R28 | Défaut. Reprise de lien refuse pendant busy. | S T08 lien initialisé avant boot, rouge puis vert. |
| R29 | Défauts de restauration tardive et boucle réseau confirmés. Contexte, serial et présence Page relus après await. hideStage invalide le chargement. Pas de lecture Page déclenchée hors connexion. | S T11/T12, races-red3, P chargement/citabilité. |
| R30 | Fenêtre durable établie par tests serveur. Le client garde maintenant la tentative avant requête. État avant journal synthétisé dans la base isolée pour preuve utilisateur. Aucun crash réel de processus sain. | tests.rs crash avant prepare, S T22, live transaction métadonnées puis reload et menu Actions puis Renvoyer. |
| R31 | État synthétique, aucun DELETE bot_pages de production trouvé. Suppression d'un fil conserve sa Page. Le cas artificiel est distingué du parcours orphelin réel. | anchors.rs suppression fixture, tests/pages.rs et renvoi serveur réel après reload dans drive-full.log. Aucune prétention de déclencheur produit. |
| R32 | Défaut de créations concurrentes. Un seul propriétaire de création par brouillon. Pièces jointes, mémo, mode voix et réunion passent par le transfert commun. Réunion garde son endpoint atomique. | S T24, A T24, V N05, VC R32, M création partagée. |

## Groupes de tests

| Groupe | Cas et preuve attendue |
| --- | --- |
| T01 | Fil d'origine supprimé, nouvel envoi 400, puce puis ancre identique au nouvel essai. S et live. |
| T02 | Même scénario 409. Validation serveur inchangée. S, live et Rust. |
| T03 | Création refusée, saisie durant await et mentions conservées. S et V. |
| T04 | DTO absents ou enregistrés dans chacun des cinq états. S, copies indépendantes. |
| T05 | Envoi réseau, refresh réussi ou échoué, rechargement puis même ID/extras. S et live. |
| T06 | Accès iPhone désactivé sur envoi ou refresh seulement, fichiers non consommés. S et iphone. |
| T07 | Sous-fil commencé avant envoi puis retour tardif, refusé ou déjà enregistré. S. |
| T08 | Bot créé tardivement et reprise de lien pendant création. S. |
| T09 | Snapshot omet A, brouillon new distinct intact, récupération affichée. S, V et N06. |
| T10 | Renommage courant à la restitution. S, P et live. |
| T11 | Passage présent/absent, Page indisponible/absente, réseau, réponses tardives, ordre modifié. S et P. |
| T12 | Ancien accusé/restauration après retrait, même passage, autre passage ou navigation Page. S, P et live. |
| T13 | Même contenu rejoint avant/après commit ancien, image et document, token durable. A et Rust. |
| T14 | Élément récent distinct, accusé reçu depuis un autre fil. A et V. |
| T15 | Erreurs import/préparation et réponses tardives après retrait, notification d'échec de libération. A. |
| T16 | Fin de tâche pendant refus, texte et fichiers retenus, mémo annoncé. S, V, Rust delegation. |
| T17 | Création mémo refusée, refresh indisponible, fil absent puis onState avant Réessayer. V. |
| T18 | État initial sans drafts, restauration tardive contre retrait/réimport/import en cours. A et Rust tokens. |
| T19 | Reprendre après rechargement, ancre originale et validation stricte. S, live React et Rust. |
| T20 | React Renvoyer serveur puis repli 404, erreurs 400/409/réseau et ancre intacte. Live et S. |
| T21 | Mentions du vrai contrat mountInput, saisie pendant attente, aller-retour, stockage indisponible, transfert de fil vide. S et live éditeur. |
| T22 | Métadonnées durables avant journal, client vivant puis reload, même tentative. Rust, S et fixture live. |
| T23 | Page réellement absente seulement par fixture, distincte d'une Page orpheline. Production non établie. Le DTO garde present=true et anchor=null. Envoi direct sans ancre refusé 409, puis renvoi serveur réel réussi après reload, dans drive-full.log. |
| T24 | Création commune et transfert pour chaque producteur du brouillon. S, A, V, VC et M. |

## Résultats de la tête intégrée

- `classe/r10-node-integrated.log`. 1 165/1 165, zéro échec, annulation, skip ou todo.
- `classe/r10-rust-integrated.log`. Lib 303, délégation 82, délégation réussie 6, Pages 34, échelle state 2 et stockage 27. 454 réussis, zéro échec, ignoré ou filtré.
- `classe/r10-web-build.log` et `classe/r10-native-build.log`. Compilations web et Cocoa réussies. Aucun bundle modifié par la reconstruction.
- `classe/r10-fmt-integrated.log` et `classe/r10-gitleaks-final.log`. Contrôles de format et de secrets avant push.

Le parcours complet Chrome termine avec sortie 0 dans `r10-final/drive-full.log`. Les 11 captures finales ont été inspectées, dont les refus 400 et 409 en clair et sombre à 1140×780 et 820×560. Doctor confirme la même tête et le même binaire avant et après. Cleanup confirme l’arrêt des seuls PID enregistrés et du pont pm-i294.

## Résultats avant intégration de main

- Shell, Pages, réunions, délégation UI et toast. `classe/r10-shell-confirmed-green.log`, 115/115.
- Pièces jointes UI. `r10-attachments/ui-green-complete.log`, 26/26.
- Rust lib, délégation et stockage. `r10-attachments/rust-green-complete.log`, 295 + 78 + 27 réussis.
- Voix. `r10-voice/all-green.log`, 85/85. Détails dans `r10-voice/matrice.md`.
- Premier sweep Node. `classe/r10-node-all.log`, 1 154 tests, 7 échecs de fixtures ensuite corrigés et rejoués dans les 115 ci-dessus. Ce premier sweep n’est pas présenté comme vert.

La reprise visible d’un message ordinaire utilise Actions puis Renvoyer. Le bouton Reprendre l’envoi n’est pas rendu pour ce statut dans Thread.jsx. Les callbacks de reprise sont couverts en tests; la preuve live utilise le menu réellement disponible.

## Précisions Astra

| Précision | Qualification et rattachement |
| --- | --- |
| N01 | Défaut. Orpheline transférée par le contrat commun utilisé séparément par fichiers, mémo, mode voix et réunion. T24. |
| N02 | Défaut confirmé. Réordonnancement de deux Pages du même fil ne déplace plus la Page citée. S T11 reorder. |
| N03 | Défaut confirmé. A absent ne doit pas restaurer sous new. S T09 avec brouillon new distinct. |
| N04 | Défaut de double création. Le même brouillon partage la promesse et les fichiers suivent son transfert. S et A T24. |
| N05 | Défaut capture récente. Session vocale transférée avant onState, ancien mémo seul consommé. V T14/T24. |
| N06 | Navigation notification déjà différée, refresh dangereux reproduit avant et pendant envoi refusé. S N06, restauration de A puis B distinct. |
| N07 | Défaut de notification écrasée. notifyDraft conserve les explications au toast final de la séquence. S T10/T11/T09 et V. |
| N08 | Conservation, hors classe citation/fichier. floating.jsx garde draft au catch et le vide seulement après succès, refresh échoué annoncé séparément. Tests floating-ui et lecture du chemin réel. |

Les principes lus sont principle-fix-root-causes, principle-model-the-domain et principle-prove-it-works. Ils ont conduit à traiter l'appartenance du brouillon et l'identité de sélection, puis à vérifier les refus sur le vrai parcours. Aucun verdict officiel n'est émis par l'owner. Le coordinateur conserve verifier et pret.
