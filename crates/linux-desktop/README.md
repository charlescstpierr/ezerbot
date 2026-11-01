# grokbot-linux-desktop

Services du bureau Linux d’Ezerbot. La crate cache zbus : aucun type de transport ne sort de son
interface publique, et elle est vide hors Linux.

- `Bus` : le seul endroit où l’on choisit un bus. En release, le bus de session ; en debug ou en
  test, rien sans `GROKBOT_TEST_SESSION_BUS=unix:path=<socket>`, accepté seulement si le socket est
  sous un HOME qui contient `.ezerbot-test-home`. Une valeur mal formée ne retombe sur aucun autre bus.
- `notifications` : `org.freedesktop.Notifications` sur une connexion longue, clic en `ActionInvoked`.
  Une alerte est identifiée par son serveur et son numéro, car un serveur redémarré réutilise les
  numéros. Le wrapper garde l’événement des 64 dernières alertes et ferme les plus anciennes. Un
  clic ouvre `ezerbot://app/v1/notification?id=<événement>`, que l’interface mène à sa carte comme
  un clic sur macOS. À la sortie propre, les alertes encore affichées sont fermées.
- `application` : instance unique et boîte de réception des liens sur `org.freedesktop.Application`
  (au plus 16 liens en attente, 2048 octets chacun, comme `app/native/main.m`).
- `scheme` : entrée `.desktop` et enregistrement de `x-scheme-handler/ezerbot` par `xdg-mime`. La
  branche générique de `xdg-open` ne sait pas lire un `Exec` entre guillemets : un exécutable dont le
  chemin contient une espace, un `%`, un `=` ou un caractère réservé passe par un petit lanceur écrit à côté
  de l’entrée. Si le dossier de données a lui-même un tel chemin, l’enregistrement est refusé.
- `open` : `xdg-open` sans shell, sans attendre l’application ouverte.
- `testing` (fonctionnalité `test-support`) : HOME isolé, `dbus-daemon` sans activation, vrai
  faux serveur de notifications.

## Branchement dans la coque

La coque garde son instance unique sur `shell.fifo`, sans `application`, qu'elle pourra remplacer.
Un lien reçu en argument passe par ce fichier jusqu'à `window.EzerbotDeepLinks.receive`. La coque
transmet les notifications à `POST /api/notifications/native` par `EzerbotShell.notifications`.
`tools/build-app-linux.sh` enregistre `ezerbot-linux` pour `ezerbot://`. Le CI installe
`dbus-daemon xdg-utils` (`desktop-file-utils` est facultatif) : s'il en manque un, les
tests concernés échouent en nommant le paquet.
