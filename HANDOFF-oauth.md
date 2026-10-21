# Handoff — connexion ChatGPT OAuth d'Ezerbot (style Hermes)

Origine : worker local `fm/gb-oauth-reel` (arrêté le 2026-10-21 ~11 h 40 heure de Toronto), base locale `6b10e5c` (= `ff208de` sur GitHub, #71).
Le diff complet (4 commits + changements non commités) s'est appliqué **proprement** sur `main` (`d4e6938`), aucun conflit.

## Objectif
Une seule connexion OpenAI : l'abonnement ChatGPT du propriétaire via OAuth (bouton « Se connecter »), même quota que partout ailleurs. Pas de clé API, pas de « Codex » séparé, pas de multicomptes.

## Causes trouvées
- Deux chemins proposés ; l'ancienne sélection `sharing` (/v1) était conservée au démarrage au lieu du backend Codex de l'abonnement ChatGPT. → une seule session de production, un seul bouton.
- Sonde parfois sur `gpt-5.5` (refusé : 400 « not supported when using Codex with a ChatGPT account ») et champs d'outils sans outils. → défaut/repli GPT-6.1 Sol, corps compatible Codex.
- Attente jusqu'à 30 s dans `launch` sous le verrou HTTP → gel de l'app pendant « Vérification » (bloquant du juge). → attente déplacée dans le worker, annulable par Arrêter.
- « Réessayer » ne relançait pas la sonde en échec (bloquant du juge). → relance la sonde avant le tour.
- SSE : commentaire/final mélangés, textes `done` perdus, verbosité forcée `low` (réponses résumées). → réconciliation sans doublon, final lu dans `response.completed`, verbosité imposée retirée ; badge « Connecté » seulement après une sonde complète avec texte visible.

## Ce qui est codé (fichiers)
`crates/wrapper/src/{provider.rs,provider/codex.rs,provider/connection.rs,provider/reliable.rs,upstream.rs,gateway.rs,features/engine.rs}`, `app/ui/{app.js,panels/usage.js}`, `app/native/main.m`, tests wrapper + UI, `docs/chat-openai-reel.md`.

## État des tests au moment de l'arrêt (checks.sh, exit 101, phase « targeted »)
- Node UI (`provider-ui.test.cjs` etc.) : 12/12 OK (dont « one ChatGPT subscription even with stale backend data »).
- Test de mise en page composer (node-first.log) : 3 échecs par timeout ~20 s (probablement environnement Chrome, à revérifier).
- `chat_reliability` : 18/20 ; échecs :
  - `connection_test_without_an_account_explains_the_required_action_without_network` : le test attend encore l'ancien texte « Connectez-vous via Codex ci-dessus. », le code dit « Connectez votre compte ChatGPT ci-dessus. » → mettre le test à jour.
  - `connection_probe_accepts_a_reasoning_token_cap_as_proof_of_reachability` : « fake chat did not settle » (tests/chat_reliability.rs:310).
  - Un run antérieur a aussi avorté (SIGABRT, panic dans un destructeur, `four_abandoned_sockets_refuse_the_next_turn_until_they_close`).
- `codex` : 8/22 ; 14 échecs, la plupart en cascade `PoisonError` (mutex empoisonné par un premier panic dans `tests/support`) — trouver le premier panic réel (lancer en `--test-threads=1` un par un).
- Non lancés : workspace complet, clippy, fmt --check, build natif.

## Reste à faire
1. Faire passer `cargo test -p grokbot-wrapper --test codex --test chat_reliability --test gateway`, puis workspace, clippy `-D warnings`, fmt, tests Node.
2. Preuve réelle (non faite) : copie de test avec HOME jetable + `EZERBOT_TEST_HOME`, un seul clic du propriétaire sur « Se connecter », puis vérifier : HTTP 200, réponse complète, modèle par défaut GPT-6.1 accepté, effort transmis, refresh, quota WHAM, déconnexion. Aucun jeton dans les journaux.
3. Sécurité : ne jamais lire/importer les jetons de ~/.codex, ~/.pi ni la valeur des jetons stockés par Ezerbot.

## Référence Hermes (NousResearch/hermes-agent @ 3f2afed3eb813cb00bd381d29b084d0e01de048f) — extraits, aucun secret
Valeurs publiques identiques à celles d'Ezerbot :
- Client OAuth public : `app_EMoamEEZ73f0CkXaXp7hrann` ; issuer `https://auth.openai.com`
- Scopes `openid profile email offline_access`, PKCE S256, callback `http://localhost:1455/auth/callback`
- Échange et refresh : `POST https://auth.openai.com/oauth/token` (form-urlencoded)
- Backend : `https://chatgpt.com/backend-api/codex` → `/responses` ; en-tête `chatgpt-account-id` (+ en-tête de résidence tiré du JWT)
- Catalogue par compte : `/backend-api/codex/models?client_version=...` (sans ChatGPT-Account-ID → `{"models":[]}` en 200)
- Quota : `/backend-api/wham/usage` (fenêtres `primary_window`/`secondary_window`, `additional_rate_limits`)

```python
# hermes_cli/auth_codex.py — refresh
response = client.post(
    CODEX_OAUTH_TOKEN_URL, headers={"Content-Type": "application/x-www-form-urlencoded"},
    data={"grant_type": "refresh_token", "refresh_token": refresh_token,
          "client_id": CODEX_OAUTH_CLIENT_ID})
if response.status_code == 429:  # quota sur le endpoint token : refresh toujours valide, ne pas forcer la reconnexion

# usage
prefix = normalized + ("/wham" if "/backend-api" in normalized else "/api/codex")
return prefix + "/usage"
headers = {"Authorization": f"Bearer {token}", "Accept": "application/json",
           "User-Agent": "codex-cli", **codex_account_headers(token)}
```
```python
# hermes_cli/models.py — catalogue
# The per-account catalog needs ChatGPT-Account-ID (else {"models":[]} with HTTP 200
# masquerades as "no models") and, for residency-enforced workspaces, the residency header.
headers = {"Authorization": f"Bearer {access_token}", **codex_account_headers(access_token)}
# supported_in_api:false décrit l'API publique, pas la dispo Codex : ne pas filtrer dessus.
```
Non repris volontairement : device-code, import de jetons d'autres outils, multicomptes. Ezerbot utilise sa propre identité (`ezerbot`, pas `pi`) et ses propres jetons.
