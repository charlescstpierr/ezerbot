# Usage et quotas

Dans **Paramètres → Usage**, Ezerbot distingue deux mesures :

- **Quota du forfait Codex** : lu chez OpenAI avec la session OAuth créée dans Ezerbot. Les fenêtres fournies par le forfait (par exemple 5 heures et une semaine) affichent le pourcentage restant et la réinitialisation en heure locale. Un avertissement apparaît dès **80 % utilisés**, dans le panneau et au-dessus du chat.
- **Tokens locaux** : entrée, sortie et total, par bot ou par source (chat, routine, groupe, délégation, entre bots). Les tokens en cache font déjà partie de l’entrée. Un enfant délégué n’est pas recompté dans son parent.

Le quota du forfait est partagé avec les autres applications Codex. Les tokens locaux ne permettent donc pas de le calculer. L’historique local couvre toutes les connexions utilisées dans cet espace, y compris les bots supprimés. Il compte les réponses principales du moteur, pas les appels auxiliaires de titres, mémoire automatique, vision ou résumés. Les exécutions sans télémétrie sont signalées comme non mesurées, jamais estimées.

## Lecture et confidentialité

`GET /backend-api/wham/usage` utilise le Bearer et l’en-tête `chatgpt-account-id` de la session Codex propre à Ezerbot. Aucun fichier d’identifiants de pi, de Codex CLI ou d’un autre outil n’est lu. Le transport ne suit pas les redirections ; toute réponse hors 2xx est une erreur. Seuls les champs validés du quota atteignent l’interface ; ni jeton, ni corps d’erreur brut ne sont exposés.

La lecture est faite en arrière-plan, coalescée et mise en cache 60 secondes. **Actualiser** peut la relancer après 5 secondes. Une réponse 401 entraîne au plus un renouvellement et une nouvelle lecture. Une déconnexion invalide les lectures en vol. Après une erreur, aucune jauge fictive n’est affichée. Une fenêtre dont la réinitialisation est échue devient seule indisponible (`null`, listée dans `quota.unavailable`) ; les autres restent affichées. Si la dernière lecture précède cette réinitialisation, le quota est relu après 5 secondes. Le partage d’abonnement, sans connexion Codex sélectionnée, indique que la jauge n’est pas disponible.

Panneau ouvert, un changement de backend, de connexion ou d’e-mail recharge l’historique. Toutes les 60 secondes, le panneau relit seulement l’usage, sans repeindre l’espace ni ce qui n’a pas changé ; cette lecture est sautée pendant un envoi ou quand la fenêtre est masquée. **Actualiser** pressé pendant un chargement est mis en file et relancé une fois à la fin.

API locale : `GET /api/usage` et `POST /api/usage/refresh`. La première renvoie la ventilation durable et le dernier état du quota, sans attendre le réseau OpenAI.

## Vérification sans compte externe

```sh
cargo test -p grokbot-wrapper --test usage --test codex --test quota --test gateway --test http
node --test app/ui/panels/usage.test.cjs
cargo clippy --workspace --all-targets -- -D warnings
```

Les tests utilisent uniquement de faux serveurs et des sessions éphémères propres à Ezerbot. Ils couvrent OAuth/renouvellement, absence de fuite d’identifiants, validation des fenêtres, erreurs, déconnexion pendant une lecture, déduplication des tokens, sources et conservation après suppression/redémarrage. Les tests UI couvrent aussi la borne de 80 %, l’actualisation du quota quand la base locale reste inactive, l’intervalle de 60 secondes, le saut pendant un envoi, la mise en file d’**Actualiser** et le rechargement après un changement de compte.

## Test de dictée Mac (opt-in)

Le test `french_audio_recognized_on_device_after_english` (crates/wrapper/tests/voice_ui.rs) est ignoré par défaut : il exige un Mac, la permission Speech et les locales en-US/fr-CA installées. Pour le lancer :

```sh
cargo test -p grokbot-wrapper --test voice_ui -- --ignored
```
