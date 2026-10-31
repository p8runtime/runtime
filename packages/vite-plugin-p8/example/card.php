<?php
// A template: $props is what render(props) was given (JSON objects arrive
// as associative arrays).
$name = htmlspecialchars($props['name'] ?? 'nobody');
$age = isset($props['born']) ? (int) date('Y') - $props['born'] : null;
?>
<article class="card">
  <h3><?= $name ?></h3>
<?php if ($age !== null): ?>
  <p>Born <?= (int) $props['born'] ?>, <?= $age ?> years ago.</p>
<?php endif ?>
  <ul>
<?php foreach ($props['tags'] ?? [] as $tag): ?>
    <li><?= htmlspecialchars($tag) ?></li>
<?php endforeach ?>
  </ul>
  <footer><?= basename(__FILE__) ?> · PHP <?= PHP_VERSION ?></footer>
</article>
