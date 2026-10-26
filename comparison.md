# Comparaison visuelle du rail Galet

Tête produit be2cedd617db0161bab4d3192170dc9369497056. Avant compilé sur eff4b4134a02c549583dff0e96a95f3a9518c325. Même jeu de cinq bots et 32 fils dans les captures latérales avant et après.

## Composition retenue

Le board réel a été rendu avec ses styles, puis sa section page-2 inspectée. Sa capture est dans references/page-2-rendered.png. Le rail reprend Conversations, Sujets et Routines en haut, Notifications et Réglages en bas. La mascotte reste au sommet. Le bouton Nouvelle conversation est neutre et la recherche est juste dessous. Les couleurs et le focus viennent des fondations Galet.

La maquette a un rail de 60 px et une liste de 250 px. L’application conserve sa largeur latérale totale de 270 px, ou 244 px sous 1100 px. Le rail occupe 48 px. Ce choix conserve la place du fil et respecte les tests du composer et de la barre du haut. Les textes restent à 11,5 px minimum. Nouvelle conversation occupe deux lignes à 820 px.

Mémoire et la section Sujets existante restent accessibles. Le rail Sujets ouvre cette vraie section et focalise Nouveau sujet. Les dialogues Routines, Notifications et Réglages gardent leur contenu actuel. La maquette ne montrait pas tous ces accès secondaires. Les tuiles Bots gardent leur rendu, leur plus, leur sélection et leur défilement.

## Avant et après

À 820×560, la carte Espace local recouvrait la liste. Elle est supprimée. Le contenu défile dans une colonne distincte du rail. Le premier fil est visible dans la capture initiale et le dernier fil est entièrement visible puis sélectionné après défilement. La preuve conserve les rectangles DOM mesurés.

Pages vide ne prend plus de place. Un second lancement isolé utilise la sphère pages et le faux fournisseur prévu pour write_page. Les captures montrent l’absence puis l’apparition d’une Page réelle. Le test du rail vérifie aussi la disparition de cette section en changeant pour un bot sans Page.

Les groupes Aujourd’hui, Hier, Cette semaine et Précédemment sont contrôlés avec des dates SQLite connues et une horloge locale déterministe dans le parcours sur le vrai wrapper. Les captures latérales ont un historique récent.

## Limites de la comparaison

Les captures du gist des fondations datent de la tête 286fd444ad828f6c085199f786551e2a1d0b0fad. Elles montrent notamment le recouvrement et la carte de compte, avec un seul bot et une autre session fournisseur. Elles ne remplacent pas notre avant sur la base exacte.

La comparaison porte sur la barre latérale. Le texte des messages, les cartes d’outils, Réfléchit, le composer, le contenu de l’accueil et la refonte des réglages ne sont pas comparés à une nouvelle maquette dans ce ticket. Le fil n’a pas été modifié. Les vues existantes sont gardées plutôt que remplacées par les futures étapes du board.
