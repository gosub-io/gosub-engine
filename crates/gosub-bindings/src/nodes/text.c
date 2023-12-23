#include "text.h"
#include "nodes.h"

void rendertree_node_text_free_data(struct node_text_t *text) {
  free(text->value);
  text->value = NULL;

  free(text->font);
  text->font = NULL;
}

const char *rendertree_node_text_get_value(const struct node_t *node) {
  if (!node)
    return NULL;

  return (const char *)node->data.text.value;
}

const char *rendertree_node_text_get_font(const struct node_t *node) {
  if (!node)
    return NULL;

  return (const char *)node->data.text.font;
}

double rendertree_node_text_get_font_size(const struct node_t *node) {
  if (!node)
    return 0.0;

  return node->data.text.font_size;
}

bool rendertree_node_text_get_bold(const struct node_t *node) {
  if (!node)
    return false;

  return node->data.text.is_bold;
}
