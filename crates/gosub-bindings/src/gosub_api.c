#include "gosub_api.h"

int8_t rendertree_init(struct rendertree_t *rendertree, const char *html) {

  rendertree->tree = gosub_rendertree_init(html);
  if (!rendertree->tree) {
    return -1;
  }

  rendertree->iterator = gosub_rendertree_iterator_init(rendertree->tree);
  if (!rendertree->iterator) {
    gosub_rendertree_free(rendertree->tree);
    return -1;
  }

  rendertree->current_node = NULL;

  rendertree->data = rendertree_node_init();
  if (!rendertree->data) {
    gosub_rendertree_iterator_free(rendertree->iterator);
    gosub_rendertree_free(rendertree->tree);
    return -1;
  }

  return 0;
}

const struct node_t *rendertree_next(struct rendertree_t *rendertree) {
  rendertree_node_free_data(rendertree->data);
  rendertree->current_node =
      gosub_rendertree_next_node(rendertree->iterator);
  if (!rendertree->current_node)
    return NULL;
  gosub_rendertree_get_node_data(rendertree->current_node, rendertree->data);
  return (const struct node_t *)rendertree->data;
}

enum node_type_e
rendertree_get_current_node_type(const struct rendertree_t *rendertree) {
  return rendertree->data->type;
}

void rendertree_free(struct rendertree_t *rendertree) {
  gosub_rendertree_iterator_free(rendertree->iterator);
  rendertree->iterator = NULL;
  gosub_rendertree_free(rendertree->tree);
  rendertree->tree = NULL;
  rendertree_node_free(&rendertree->data);
  rendertree->data = NULL;
}
