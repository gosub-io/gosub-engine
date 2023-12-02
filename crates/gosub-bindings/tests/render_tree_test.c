#include "gosub_api.h"
#include <assert.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

int main() {
  const char *html = "<html>"
                     "<h1>this is heading 1</h1>"
                     "<h2>this is heading 2</h2>"
                     "<h3>this is heading 3</h3>"
                     "<h4>this is heading 4</h4>"
                     "<h5>this is heading 5</h5>"
                     "<h6>this is heading 6</h6>"
                     "<p>this is a paragraph</p>"
                     "</html>";
  struct render_tree_t render_tree;
  assert(render_tree_init(&render_tree, html) == 0);

  const struct node_t *node = NULL;

  // <html>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_ROOT);

  const double tol = 0.00001;

  // TODO: it'll be good at some point in the future to have the
  // margins to compute the expected_y position instead of manually
  // doing math. This will make the tests more robust if we change
  // margins/etc. in the engine.

  // <h1>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 1") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 37.0) < 0.00001);
  assert(node->data.text.is_bold == true);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 10.72) < tol);

  // <h2>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 2") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 27.5) < 0.00001);
  assert(node->data.text.is_bold == true);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 68.4) < tol);

  // <h3>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 3") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 21.5) < 0.00001);
  assert(node->data.text.is_bold == true);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 115.22) < tol);

  // <h4>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 4") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 18.5) < 0.00001);
  assert(node->data.text.is_bold == true);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 156.72) < tol);

  // <h5>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 5") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 15.5) < 0.00001);
  assert(node->data.text.is_bold == true);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 196.949) < tol);

  // <h6>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is heading 6") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 12.0) < 0.00001);
  assert(node->data.text.is_bold == true);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 236.027) < tol);

  // <p>
  node = render_tree_next(&render_tree);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(node->data.text.value, "this is a paragraph") == 0);
  assert(strcmp(node->data.text.font, "Times New Roman") == 0);
  assert(fabs(node->data.text.font_size - 18.5) < 0.00001);
  assert(node->data.text.is_bold == false);
  assert(fabs(node->position.x - 0.00) < tol);
  assert(fabs(node->position.y - 268.516) < tol);

  // end of iterator, last node is free'd
  node = render_tree_next(&render_tree);
  assert(node == NULL);

  render_tree_free(&render_tree);

  printf("\033[0;32mrender_tree_test.c: All assertions passed\n\033[0;30m");
  return 0;
}
