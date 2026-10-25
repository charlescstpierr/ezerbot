# Une connexion ChatGPT, des réponses réellement vérifiées

## Parcours de production

- Un seul bouton « Se connecter », un seul compte, aucune clé API et aucun choix de deuxième abonnement. Les anciens fichiers de sélection et de partage ne sont ni utilisés ni modifiés. Une ancienne installation utilisant seulement le partage doit autoriser ChatGPT une fois dans le nouveau parcours ; son historique reste présent.
- OAuth authorization-code + PKCE S256 : client public `app_EMoamEEZ73f0CkXaXp7hrann`, scopes `openid profile email offline_access`, `https://auth.openai.com/oauth/authorize`, retour `http://localhost:1455/auth/callback`, échange et renouvellement via `https://auth.openai.com/oauth/token`. Le port enregistré ne peut pas varier. L'état aléatoire est vérifié ; Annuler empêche toute écriture tardive.
- La session appartient à Ezerbot : fichier privé atomique, mode 0600, rotation sérialisée du refresh token. Aucun jeton ou catalogue de Codex CLI, pi ou d'un autre logiciel n'est importé.
- Appels à `https://chatgpt.com/backend-api/codex/responses` : Bearer, `chatgpt-account-id` et, si présent dans le JWT, `x-openai-internal-codex-residency`. Identité de l'application : `originator: ezerbot`, `User-Agent: ezerbot/0.1.0`.
- Catalogue du compte via `/codex/models?client_version=99.0.0`, avec les mêmes en-têtes de compte. Si indisponible, catalogue de repli connu. Préférence de départ : GPT-6.1 Sol (`gpt-6.1-sol`). Un catalogue disponible reste prioritaire ; un compte ne reçoit pas artificiellement un modèle absent. Effort choisi transmis sans transformation.
- `signed_in` indique une session enregistrée, pas une connexion prouvée. Le badge « ChatGPT » exige une sonde Responses terminée avec du texte visible. Une sonde vide, incomplète ou refusée n'est pas un succès. Le test explicite accepte `{ "refresh": true }` pour vérifier le renouvellement de la seule session Ezerbot, sans exposer ses jetons.
- Le quota vient de `/backend-api/wham/usage` pour ce compte. Il s'agit des fenêtres Codex de cet abonnement ChatGPT, partagées avec les applications qui utilisent ce quota ; Ezerbot ne peut pas fusionner les limites que fixe OpenAI pour ses différents produits.
- La déconnexion retire les jetons locaux de cette session uniquement. Aucun endpoint de révocation Codex non documenté n'est inventé.

## Causes et correctifs

1. L'ancienne sélection persistée pouvait laisser l'application sur le protocole de partage `/v1`, alors que l'interface proposait aussi « Codex » comme autre connexion. La production utilise désormais une seule session ChatGPT/Codex. L'injection explicite `/v1` sur loopback reste uniquement un harnais de tests de compatibilité.
2. Le repli de sonde `gpt-5.5` était refusé par un compte ChatGPT ; il utilise désormais `gpt-6.1-sol`. Les champs `tool_choice` et `parallel_tool_calls` sont absents sans outils.
3. L'attente de sonde ne retient plus la question. Le badge « ChatGPT » attend toujours une sonde réussie, mais l'envoi part tout de suite : l'état, Arrêter et un second envoi restent disponibles pendant la vérification. Un échec de sonde n'empêche pas le tour ; un refus réel d'OpenAI reste une erreur de fil réessayable.
4. « Réessayer » relance une sonde en échec en arrière-plan, sans attendre son résultat avant l'appel de chat.
5. Le flux mélangeait le commentaire Codex et la réponse finale, et ignorait les réponses présentes seulement dans les événements `done`. Les items sont réconciliés sans doublon ; le texte final peut aussi provenir de l'enveloppe `response.completed`. Les résumés de raisonnement ne deviennent pas la réponse. Le forçage global de `text.verbosity: low` est supprimé.
6. Les sockets abandonnés sont comptés dès l'annulation, pour éviter de dépasser la limite pendant leur fermeture.

## Référence Hermes

Comparaison avec NousResearch/hermes-agent, commit `3f2afed3eb813cb00bd381d29b084d0e01de048f` : `hermes_cli/auth_constants.py`, `auth_codex_browser.py`, `auth_codex.py`, `codex_models.py`, `agent/codex_headers.py` et `agent/transports/codex.py`.

Même client public, scopes, PKCE, callback, échange, refresh, endpoint et en-têtes de compte. Hermes privilégie par défaut le device-code et propose aussi le navigateur ; Ezerbot garde uniquement le navigateur. Hermes utilise sa propre identité réseau, et Ezerbot fait de même au lieu d'annoncer `pi`. Hermes a des mécanismes d'import de sessions d'autres outils : ils ne sont pas repris. Le catalogue du compte et l'en-tête de résidence ont été alignés sur sa référence.

## Validation sans secrets

Tests : `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, tests Node dans `app/web/src`, `app/ui` et `app/ui/panels`.

Les régressions ciblées couvrent OAuth/PKCE/rotation/0600, choix unique et ancienne sélection ignorée, sonde lente qui n'empêche ni l'envoi ni la réponse, relance après échec de sonde, texte complet et effort réel, réconciliation SSE et fermeture des lectures annulées.

La preuve avec un véritable compte se lance sur le Mac du propriétaire avec `tools/preuve-chatgpt-mac.sh`. Construisez d'abord le wrapper avec le verrou Cargo prévu sur votre machine. `EZERBOT_WRAPPER` peut désigner ce binaire. Sinon, le script utilise celui du bundle `EZERBOT_APP`, par défaut `app/build/Ezerbot.app`.

Le script compile `tools/proof-chatgpt-host.m`, qui importe la vraie coque Cocoa sans changer l'application distribuée. Il crée un HOME privé sous `TMPDIR`, pose `EZERBOT_TEST_HOME` et `CFFIXED_USER_HOME` égaux à ce HOME et ouvre uniquement cette copie. La coque génère son identité de lancement et authentifie elle-même les requêtes. Le pilote reçoit les commandes par un pipe anonyme. Il renvoie seulement les résultats contrôlés, sans transmettre l'identité à Python, curl ou un serveur de diagnostic. Il refuse les redirections. Les réponses sont contrôlées en mémoire avant toute sortie. Le mode réel refuse un faux fournisseur et n'importe aucune variable d'identifiant du parent.

Charles clique une seule fois sur « Se connecter », sauf si une session de test déjà signée est encore dans la copie conservée. Le script exige chaque statut 200, la réponse complète, GPT-6.1 Sol et l'effort high. Le renouvellement doit changer un jeton enregistré et produire un nouvel événement de sauvegarde dans le journal de ce lancement. Le quota WHAM doit avoir une fenêtre numérique comprise entre 0 et 100. Après déconnexion, les deux indicateurs de connexion doivent être faux et les trois jetons absents du fichier privé en mode 0600. Les journaux moteur et coque sont contrôlés pour les motifs historiques et les valeurs exactes connues, sans les afficher. Seule la session OAuth du HOME jetable persiste pour permettre ces contrôles. L'identité de lancement reste en mémoire.

Un échec conserve la copie privée pour la reprise. Un succès la supprime, sauf avec `--keep`. `--self-check` teste les assertions et leurs contre-exemples, puis rejoue les appels contre le garde local sans joindre OpenAI. `--guard-calls` vérifie seulement ce transport. Ces deux modes annoncent explicitement que le parcours réel n'a pas été exécuté. La preuve d'interface à faux fournisseur reste une commande distincte, `.cursor/skills/verify-ezerbot/scripts/prove.sh`. Notifications et services natifs de debug restent factices. Ne jamais ouvrir la copie dans les données réelles, importer de jetons d'autres outils, afficher un fichier d'authentification ou enregistrer les codes OAuth.

Les journaux ne contiennent que le statut HTTP, des codes fixes, un digest de tour et les options modèle/effort validées. Aucune valeur d'en-tête, aucun jeton, aucun prompt ou texte de réponse n'y est écrit par ce diagnostic.
