#include "gosub_api.h"

int8_t render_tree_init(struct render_tree_t *render_tree, const char *html) {

  render_tree->tree = gosub_render_tree_init(html);
  if (!render_tree->tree) {
    return -1;
  }

  render_tree->iterator = gosub_render_tree_iterator_init(render_tree->tree);
  if (!render_tree->iterator) {
    gosub_render_tree_free(render_tree->tree);
    return -1;
  }

  render_tree->current_node = NULL;

  render_tree->data = render_tree_node_init();
  if (!render_tree->data) {
    gosub_render_tree_iterator_free(render_tree->iterator);
    gosub_render_tree_free(render_tree->tree);
    return -1;
  }

  return 0;
}

const struct node_t *render_tree_next(struct render_tree_t *render_tree) {
  render_tree_node_free_data(render_tree->data);
  render_tree->current_node =
      gosub_render_tree_next_node(render_tree->iterator);
  if (!render_tree->current_node)
    return NULL;
  gosub_render_tree_get_node_data(render_tree->current_node, render_tree->data);
  return (const struct node_t *)render_tree->data;
}

enum node_type_e
render_tree_get_current_node_type(const struct render_tree_t *render_tree) {
  return render_tree->data->type;
}

void render_tree_free(struct render_tree_t *render_tree) {
  gosub_render_tree_iterator_free(render_tree->iterator);
  render_tree->iterator = NULL;
  gosub_render_tree_free(render_tree->tree);
  render_tree->tree = NULL;
  render_tree_node_free(&render_tree->data);
  render_tree->data = NULL;
}
