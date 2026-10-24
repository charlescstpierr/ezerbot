# Mémoire automatique — F1 hors ligne sur sorties enregistrées

Ce banc rejoue 64 messages **synthétiques en français** et des réponses brutes figées au format de sortie du modèle. Les réponses sont des **simulations écrites à la main**, pas des captures d’un vrai modèle : elles comprennent des abstentions, omissions, reformulations, propositions ponctuelles, doublons et pièges. Le score vérifie le banc et les filtres ; il ne mesure **pas** la qualité d’un modèle réel et n’est pas comparable à AlpsBench.

## Rejouer

Depuis la racine, sans compte, réseau (même loopback), serveur HTTP ni lancement de l’application :

```sh
cargo test -p grokbot-wrapper --locked --lib features::memory_auto::evaluation -- --nocapture
cargo test -p grokbot-wrapper --locked --test memory_auto
cargo clippy -p grokbot-wrapper --locked --lib --test memory_auto -- -D warnings
```

Sur le Mac partagé, préfixer chaque commande Cargo par le script de limitation des builds. Les tests créent uniquement des bases/coffres neufs en dossier temporaire, nettoyés après usage. Ils n’appellent ni `tick` ni une API d’envoi : le fournisseur factice est une fonction retournant le texte enregistré, directement à la frontière de `extract`. En production, cette même frontière appelle toujours `Provider::extract_memory` ; le prompt, les constantes, la sélection du modèle et les garde-fous ne changent pas.

## Données versionnées

- `crates/wrapper/tests/fixtures/memory_auto/corpus.json` : `id`, `message`, `expected` (citations nouvelles à retenir, éventuellement vide), et listes facultatives `known`, `secret_traps`, `vault_secrets`.
- `crates/wrapper/tests/fixtures/memory_auto/recorded_outputs.json` : mêmes `id`, `output` (**texte JSON brut**, pas une reconstruction des étiquettes), `called` (la frontière fournisseur doit-elle être atteinte ?).
- `crates/wrapper/src/features/memory_auto/evaluation.rs` : calculateur, rejeu et compteurs.

Un message est évalué dans une base neuve, avec ses seuls faits `known` et ses secrets factices de coffre. Les étiquettes attendent seulement les faits **nouveaux**, pas un fait déjà connu. Les identifiants des deux fichiers doivent correspondre un à un. Les messages exclus avant appel conservent une réponse simulée inutilisée ; `called: false` vérifie cette exclusion. Les propositions répétées dans une réponse et les doublons de faits préchargés sont tous deux exercés. Les faux jetons, mots de passe, numéros, identifiants et adresses ne correspondent à aucun compte ; ne jamais remplacer ces fixtures par des données personnelles.

## Métrique et garde-fous

L’appariement est un à un, après le **même `normalize` que l’extracteur** (minuscules et espaces condensés, ponctuation conservée). Une prédiction répétée ne gagne pas deux faits justes. Le score porte sur les faits réellement **écrits**, après les filtres, et non sur les propositions brutes :

- P = faits justes / faits écrits ; R = faits justes / faits attendus ; F1 = 2 × justes / (écrits + attendus).
- Agrégation **micro** sur les faits du corpus. Un message sans attendu et sans fait écrit est juste (`correct_empty`) sans ajouter de faux vrai positif au total de faits.
- Un ensemble vide des deux côtés donne P = R = F1 = 1. Une seule liste vide donne 0 pour les trois scores. Trois cas calculés à la main vérifient exactement les scores : parfait normalisé, partiel (P = 1/2, R = 1/3, F1 = 2/5), vide/vide.

Indépendamment du F1, le rejeu échoue dès qu’un des compteurs suivants n’est pas nul :

1. **Citations inexactes écrites** : le texte écrit n’est pas une sous-chaîne exacte du message original (sans normalisation).
2. **Secrets/identifiants retenus** : un texte écrit contient un `secret_traps` étiqueté indépendamment des filtres de production. Un piège lexicalement ordinaire enregistré au coffre vérifie aussi la rédaction du coffre.
3. **Doublons créés** : même texte normalisé qu’un fait connu ou qu’un autre fait écrit pour ce message.

Le contrôle lit tous les faits automatiques persistés, pas seulement les actifs. Un test de sensibilité écrit volontairement un secret-piège en base en contournant `extract`, puis prouve que la même porte de validation le refuse, même avec une citation exacte. D’autres contrôles vérifient citations inventées et doublons. Aucun assouplissement de `unsafe_text`, du coffre ou de la citation exacte n’est permis pour améliorer le score.

## Départ reproductible (fixtures v1)

| Mesure | Valeur |
| --- | ---: |
| Messages / appels factices | 64 / 47 |
| Faits justes / écrits / attendus | 30 / 33 / 42 |
| Messages vides de faits correctement traités | 26 |
| P | 30/33 = 0,909090909 |
| R | 30/42 = 0,714285714 |
| F1 | 60/75 = 0,800000000 |
| Citations inexactes / secrets / doublons | 0 / 0 / 0 |

Le rejeu imprime les compteurs et scores avec `--nocapture`. Les compteurs de départ sont figés pour la reproductibilité, **pas** comme seuil de qualité d’un modèle réel. Une évolution volontaire des données doit documenter son nouveau départ. Un lanceur de vrai modèle, des messages privés, les essais de prompt/constantes ou un changement de modèle relèvent du ticket suivant, pas de ce banc.
