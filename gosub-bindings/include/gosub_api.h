#ifndef GOSUB_API_H
#define GOSUB_API_H

#include "nodes.h"

extern void *gosub_render_tree_init(const char *html);
extern void *gosub_render_tree_iterator_init(void *render_tree);
extern const void *gosub_render_tree_next_node(void *tree_iterator);
extern void gosub_render_tree_get_node_data(const void *current_node,
                                            struct node_t *node);
extern void gosub_render_tree_iterator_free(void *tree_iterator);
extern void gosub_render_tree_free(void *render_free);

struct render_tree_t {
  void *tree;
  void *iterator;
  const void *current_node;
  struct node_t *data;
};

/// Construct a new render tree.
/// Returns NULL if unsuccessful.
struct render_tree_t *render_tree_init(const char *html);

/// Get the next node in the render tree as a read-only pointer.
/// Returns NULL when reaching end of tree.
const struct node_t *render_tree_next(struct render_tree_t *render_tree);

/// Get the type of the current node the render tree is pointing to.
enum node_type_e
render_tree_get_current_node_type(const struct render_tree_t *render_tree);

/// Free all memory tied to the render tree
void render_tree_free(struct render_tree_t **render_tree);

#endif
