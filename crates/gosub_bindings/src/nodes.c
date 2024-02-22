#include "nodes.h"

struct node_t *rendertree_node_init() {
  struct node_t *node = malloc(sizeof(*node));
  if (!node)
    return NULL;

  node->type = NODE_TYPE_ROOT;
  node->data.root = true; // dummy value

  return node;
}

void rendertree_node_free_data(struct node_t *node) {
  switch (node->type) {
  case NODE_TYPE_ROOT:
    break;
  case NODE_TYPE_TEXT:
    rendertree_node_text_free_data(&node->data.text);
    break;
  }
}

double rendertree_node_get_x(const struct node_t *node) {
  return node->position.x;
}

double rendertree_node_get_y(const struct node_t *node) {
  return node->position.y;
}

double rendertree_node_get_margin_top(const struct node_t *node) {
  return node->margin.top;
}

double rendertree_node_get_margin_left(const struct node_t *node) {
  return node->margin.left;
}

double rendertree_node_get_margin_right(const struct node_t *node) {
  return node->margin.right;
}

double rendertree_node_get_margin_bottom(const struct node_t *node) {
  return node->margin.bottom;
}

double rendertree_node_get_padding_top(const struct node_t *node) {
  return node->padding.top;
}

double rendertree_node_get_padding_left(const struct node_t *node) {
  return node->padding.left;
}

double rendertree_node_get_padding_right(const struct node_t *node) {
  return node->padding.right;
}

double rendertree_node_get_padding_bottom(const struct node_t *node) {
  return node->padding.bottom;
}

void rendertree_node_free(struct node_t **node) {
  free(*node);
  *node = NULL;
}
