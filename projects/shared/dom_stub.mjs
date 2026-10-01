globalThis.document = {
  body: {
    append(element) {
      console.log(element.textContent);
    },
    insertAdjacentHTML(_, html) {
      console.log(html);
    },
  },
  createElement() {
    return { textContent: "" };
  },
  getElementById() {
    return null;
  },
};
