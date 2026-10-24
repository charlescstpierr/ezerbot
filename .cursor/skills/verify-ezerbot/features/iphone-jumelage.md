# Jumelage iPhone par QR

Dans Paramètres › iPhone, Créer un QR de jumelage affiche un QR et le même code. Le QR contient l’adresse HTTPS Tailscale du Mac et le code dans le fragment (`#jumelage=<code>`). Scanné avec l’appareil photo, il ouvre Ezerbot avec l’adresse et le code remplis ; la confirmation sur le Mac reste obligatoire. Cette sphère capture la carte du Mac, jamais le QR ni le code en clair.

## Sub-features

- `avant` montre la section iPhone, accès activé et Tailscale vérifié, avec le bouton Créer un QR de jumelage et sans code.
- `qr` montre la carte après ce bouton : QR, consigne de scan, code et échéance. Le QR est remplacé par un carré « QR masqué » et le code par des points avant la capture et le texte visible.

## How to get to it (user POV)

- Paramètres › iPhone, activer l’accès, Vérifier Tailscale, puis Créer un QR de jumelage.

## Driving it with chrome-devtools-axi

Preconditions:

- `launch.sh --sphere iphone-jumelage` : le HOME jetable reçoit un faux Tailscale (`mac-exemple.tailnet-exemple.ts.net`) et le port privé iPhone est un port libre. Aucun vrai Tailscale n’est appelé.
- `doctor: ok`, la pilule Modèle `#composer-tools [data-picker="model"]` est présente.

- **Avant.** L’accès est coupé puis rétabli par la bascule `#settings-iphone input[role="switch"]`, ce qui efface le code d’un passage précédent. Vérifier Tailscale affiche `.iphone-address`.
- **QR.** Le bouton pose `.iphone-pairing-card` avec `img.iphone-qr` et `.iphone-code`. Le pilote attend l’image chargée, la remplace par le carré masqué, remplace le code, puis vérifie que le code n’est plus dans `document.body.innerText`.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere iphone-jumelage`. Les huit PNG (`avant-` et `qr-`, deux thèmes, deux tailles) existent.

## Gotchas

- Le scan réel (appareil photo, Safari, préremplissage) n’est pas joué ici : il est prouvé par `crates/wrapper/tests/iphone_qr.rs`, qui relit le QR servi, et par `app/ui/iphone-panel.test.cjs`.
- Chaque passage crée un nouveau code jetable dans la base du run ; il n’est jamais imprimé.
