# Répondre en fil

« Répondre en fil », dans le menu d’un message, ouvre un sous-fil lié à ce message. Un bandeau au-dessus du sous-fil cite le message parent et ramène au fil principal. Le bot reçoit le message parent comme contexte de chacune de ses réponses dans le sous-fil. Le fil principal ne change pas.

## Sub-features

- `avant` montre le fil principal : la question « Quelle est la capitale du Canada ? » et la réponse du faux bot, sans bandeau.
- `menu` montre le menu du message de la question ouvert, avec « Répondre en fil » sous Copier.
- `sous-fil` montre le sous-fil ouvert par cette entrée : titre « Fil : … », bandeau « Fil lié à un message » qui cite « Vous : « Quelle est la capitale du Canada ? » », bouton « Voir dans le fil principal », fil vide.
- `reponse` envoie « Et en un mot ? » dans le sous-fil : la réponse du faux bot reprend le message parent (« En tenant compte du message parent : … »), et le fil principal ne contient pas cette question.
- `principal` clique « Voir dans le fil principal » : le bandeau disparaît et la question est toujours dans le fil principal.

## How to get to it (user POV)

- Survoler un message, ouvrir « Actions du message » (les trois points), choisir « Répondre en fil ». Une seconde fois sur le même message rouvre le même sous-fil. L’entrée n’existe pas sur un envoi qui n’est pas parti, sur une réponse en cours ou ratée, ni dans un groupe ou une sous-tâche.

## Driving it with chrome-devtools-axi

Preconditions:

- `doctor: ok`, badge `ChatGPT`.

- **Conversation.** Le parcours crée une conversation par l’API, envoie la question et attend la réponse du faux bot.
- **Menu.** `.gb-message[data-message-id=<question>] button[aria-label="Actions du message"]`, puis l’entrée `[role="menuitem"]` « Répondre en fil » dans `[role="menu"][aria-label="Actions du message"]`.
- **Sous-fil.** `#thread-banner` visible, `#thread-banner-quote` cite la question, `#conversation-title` commence par « Fil : ».
- **Retour.** `#thread-banner-open` ramène au fil principal.
- **Parcours groupé.** Run `bash .cursor/skills/verify-ezerbot/scripts/drive.sh --evidence <preuves> --sphere reponse-fil`. Les vingt PNG (`avant-`, `menu-`, `sous-fil-`, `reponse-` et `principal-`, deux thèmes, deux tailles) existent.

## Gotchas

- Le faux OpenAI reconnaît le message parent dans les instructions (bloc de données externes) et le cite : c’est ce qui prouve que le bot le reçoit. Le test Rust `le_sous_fil_cite_le_parent_et_le_fil_principal_garde_le_message` le prouve aussi.
- La question du sous-fil est envoyée par l’API : le champ de saisie est le même que celui des autres conversations.
- Chaque passage crée sa propre conversation dans la base jetable.
