#include "nodes.h"

struct node_t *render_tree_node_init() {
  struct node_t *node = malloc(sizeof(*node));
  if (!node)
    return NULL;

  node->type = NODE_TYPE_ROOT;
  node->data.root = true; // dummy value

  return node;
}

void render_tree_node_free_data(struct node_t *node) {
  switch (node->type) {
  case NODE_TYPE_ROOT:
    break;
  case NODE_TYPE_TEXT:
    render_tree_node_text_free_data(&node->data.text);
    break;
  }
}

void render_tree_node_free(struct node_t **node) {
  free(*node);
  *node = NULL;
}
