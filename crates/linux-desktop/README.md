# grokbot-linux-desktop

Services du bureau Linux d’Ezerbot. La crate cache zbus : aucun type de transport ne sort de son
interface publique, et elle est vide hors Linux.

- `Bus` : le seul endroit où l’on choisit un bus. En release, le bus de session ; en debug ou en
  test, rien sans `GROKBOT_TEST_SESSION_BUS=unix:path=<socket>`, accepté seulement si le socket est
  sous un HOME qui contient `.ezerbot-test-home`. Une valeur mal formée ne retombe sur aucun autre bus.
- `secrets` : collection par défaut de Secret Service (GNOME Keyring, KWallet), session chiffrée
  (DH). Elle ne déverrouille jamais : verrouillée, elle est indisponible. Si le service demande de
  confirmer une écriture ou une suppression, l’attente est bornée à 10 s, puis l’opération échoue.
- `notifications` : `org.freedesktop.Notifications` sur une connexion longue, clic en `ActionInvoked`.
- `application` : instance unique et boîte de réception des liens sur `org.freedesktop.Application`
  (au plus 16 liens en attente, 2048 octets chacun, comme `app/native/main.m`).
- `scheme` : entrée `.desktop` et enregistrement de `x-scheme-handler/ezerbot` par `xdg-mime`.
- `open` : `xdg-open` sans shell, sans attendre l’application ouverte.
- `testing` (fonctionnalité `test-support`) : HOME isolé, `dbus-daemon` sans activation, vrai
  `gnome-keyring-daemon`, faux serveur de notifications.

## Branchement dans la coque (#605)

1. Dépendre de la crate : `grokbot-linux-desktop = { workspace = true }` sous
   `[target.'cfg(target_os = "linux")'.dependencies]`.
2. Au démarrage, `application::deep_links(std::env::args_os().skip(1))` puis
   `Instance::claim(&Bus::for_app()?)`. `Secondary(remote)` : `remote.open(&liens)` (ou
   `remote.activate()` sans lien) puis sortir. `Primary(inbox)` : lire `inbox.try_recv()` et appeler
   `window.EzerbotDeepLinks.receive(url)` quand l’interface est prête ; `Received::Activate` ramène
   la fenêtre au premier plan.
3. Dans `tools/build-app-linux.sh`, une ligne :
   `grokbot-wrapper --register-url-handler /chemin/vers/l-executable [/chemin/icone.png]`. Elle écrit
   `ca.ezerbot.Ezerbot.desktop` dans `$XDG_DATA_HOME/applications` et vérifie le résultat.
4. Dans `local-api.js`, un shim qui transmet `window.webkit.messageHandlers.ezerbotNotifications`
   à `POST /api/notifications/native` (`status`, `authorize`, `presence`, `deliver`). Réponse
   `disabled` si aucun bus n’est permis, `granted` ou `error`, puis `delivered` ou `error`.
5. Ouvrir les liens externes et les documents avec `open::launch(cible)`.
6. Paquets que le CI doit installer pour que les tests tournent : `dbus-daemon gnome-keyring
   xdg-utils` (`desktop-file-utils` est facultatif). Sans eux, avec `CI` défini, les tests
   concernés affichent `SKIP` ; sans `CI`, ils échouent en nommant le paquet.
