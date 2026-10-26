# Montages et chemins de la ronde 1

Tête produit be2cedd617db0161bab4d3192170dc9369497056.
Base eff4b4134a02c549583dff0e96a95f3a9518c325.

| Chemin ou montage | Contrat préservé ou ajouté | Contrôle |
| --- | --- | --- |
| index.html, #sidebar, .sidebar-rail, .sidebar-content | Cinq noms accessibles, boutons natifs, rail fixe et contenu défilant | sidebar-rail.test.cjs et proof-rail.py, captures Mac |
| app.js, sidebarActions.conversations | Ferme Sujets et Mémoire via onNavigate, conserve le fil choisi, focalise la liste | sidebar-rail.test.cjs, preuve native clavier |
| app.js, sidebarActions.topics et #sidebar-topics | Ouvre la vraie section Sujets et focalise Nouveau sujet, sans sujet factice | sidebar-rail.test.cjs, topics.test.cjs, formulaire réel en preuve |
| routines.js, openSidebar(context) | Même dialogue Routines, bot courant préselectionné | sidebar-rail.test.cjs, routines.test.cjs, capture et Échap |
| notifications.js, openSidebar | Même clic Notifications et même dialogue que la barre du haut | sidebar-rail.test.cjs, notifications-ui.test.js, capture |
| #settings-button et #settings-dialog | Réglages existants et liens profonds inchangés | sidebar-rail.test.cjs, deep-link-navigation.test.js, drive accueil |
| syncSidebarRail | Actif issu des dialogues réels et de la page Sujet, retour au contenu après fermeture | sidebar-rail.test.cjs, preuve native clavier et Échap |
| memory-space.js, bots.parentElement | Entrée Mémoire sous recherche, avant Bots, dans le nouveau conteneur | memory-space.test.cjs et autres tests mémoire, accès live |
| pages.js, paintList | Section cachée à zéro Page et montrée pour le bot qui possède une Page | sidebar-rail.test.cjs avec vrai write_page, pages.test.cjs et deux parcours navigateur Pages |
| app.js, renderList | Aujourd’hui, Hier, Cette semaine, Précédemment selon dates locales | Fixture SQLite indépendante et horloge déterministe dans sidebar-rail.test.cjs |
| #new-chat, #search | Même création, même bot courant, raccourcis et filtrage | select-bot.test.cjs inchangé, sidebar-rail.test.cjs et touches natives en preuve |
| #sidebar-bot-list et [data-new-bot] | Même rendu et mêmes sélections, tuiles non modifiées | select-bot.test.cjs inchangé, plusieurs bots et vrai éditeur en preuve |
| styles.css et topics.css | Suppression de la carte de compte et des anciens conteneurs sticky, largeur totale conservée | dernier fil visible et sélectionnable à 820×560, contrôle des textes, composer-bar.test.cjs complet |
| .cursor/skills/verify-ezerbot/scripts/drive.py | Accueil vérifie le nom accessible du lien de marque iconographique | Drive accueil Mac sur la tête exacte, sans retirer le contrôle de marque |
| wrapper-chrome-fixture.cjs | Ajout de write_page, taille explicite et accès à sa seule base jetable | sidebar-rail.test.cjs sur vrai wrapper et faux fournisseur, cleanup enregistré par la fixture |

Aucune requête produit, route serveur, carte du fil, app/web/src/Thread.jsx, chat.js ou chat.css modifiés. Les accès iPhone conservent leur enregistrement existant. Aucun test ignoré, sauté ou assertion existante affaiblie.

La preuve est séparée du code produit. Les fichiers run.json, les marqueurs signés, les profils, les bases et les jetons ne sont jamais publiés.
