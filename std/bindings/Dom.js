export function append_text(tag, text) {
  const element = document.createElement(tag);

  element.textContent = text;
  document.body.append(element);
}

export function append_html(html) {
  document.body.insertAdjacentHTML("beforeend", html);
}

export function has(id) {
  return document.getElementById(id) !== null;
}

export function set_html(id, html) {
  document.getElementById(id).innerHTML = html;
}
