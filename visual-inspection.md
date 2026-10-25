# Inspection des captures

La série `final5` porte sur `caaf0734037a4bd3536e291d365b111f8c9ed219`. Ses 24 PNG ont été ouverts et inspectés. Chaque état existe en clair et en sombre, à 1140×780 et 820×560. Les fichiers voisins donnent le texte, l’action, le thème, la taille et les observations du DOM. `captures.json` donne les dimensions et les empreintes des PNG.

Les captures montrent le panneau sans commentaire, les deux repères, la liste ouverte, le prix modifié avec Revenir à avant, la restauration à 19 €, le passage 21 € traité sans repère et la Page dont le fil d’origine a disparu. Le champ Renommer reste lisible après le correctif de largeur. À petite taille, la liste et le canevas défilent séparément. Un passage hors du canevas visible ne reçoit pas de pastille flottante. Aucun débordement horizontal de la fenêtre n’a été détecté.

Deux références de la PR Galet #272 ont été téléchargées et ouvertes.

- [Conversation sombre](https://github.com/user-attachments/assets/def69ed0-1f54-48d0-a404-08f7a14dd0f8).
- [Accueil clair](https://github.com/user-attachments/assets/2288056d-bbb5-4cad-8bdc-4e2f63d50b0c).

La comparaison visuelle porte sur les fonds, la barre latérale, les bordures, les boutons neutres, les cartes et la typographie. Le nouveau panneau réutilise ces styles et les variables Galet. Les deux thèmes mesurent respectivement `--canvas=#0e0f12` et `--canvas=#ffffff`. Le choix du thème est relu dans les Paramètres après réouverture.

Ces références ne présentent pas les nouveaux contrôles de commentaires. Aucune parité pixel à pixel avec ces nouveaux contrôles n’est revendiquée. `compare.sh` inventorie les corpus reports, shots et annex présents, sans capture appariée par nom. Ces corpus ne sont pas comparés exhaustivement. Les corpus evidence et artifacts/formatted sont absents. Leur absence reste déclarée dans `final5/deviations.md`.

Le run `final4` avait signalé le champ Renommer trop étroit et produit un verdict owner FIX. Le test desktop reproduit ce défaut avant la correction, puis passe avec la base flex de 140 px. Ses reçus et sa capture restent conservés.
