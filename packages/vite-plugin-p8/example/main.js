// A PHP template as a module: render(props) is the HTML it prints, with
// the props as $props.
import card from './card.php';

const people = [
  { name: 'Ada Lovelace', born: 1815, tags: ['math', 'engines'] },
  { name: 'Rasmus Lerdorf', born: 1968, tags: ['php'] },
];

const root = document.getElementById('cards');
for (const person of people) {
  root.insertAdjacentHTML('beforeend', await card.render(person));
}
