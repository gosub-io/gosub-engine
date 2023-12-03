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
  struct rendertree_t rendertree;
  assert(rendertree_init(&rendertree, html) == 0);

  const struct node_t *node = NULL;

  // <html>
  node = rendertree_next(&rendertree);
  assert(node->type == NODE_TYPE_ROOT);

  const double tol = 0.00001;

  double y = 0.00;

  // <h1>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is heading 1") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 37.0) < tol);
  assert(rendertree_node_text_get_bold(node) == true);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);
  y += (rendertree_node_text_get_font_size(node) + rendertree_node_get_margin_bottom(node));

  // <h2>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is heading 2") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 27.5) < tol);
  assert(rendertree_node_text_get_bold(node) == true);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);
  y += (rendertree_node_text_get_font_size(node) + rendertree_node_get_margin_bottom(node));

  // <h3>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is heading 3") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 21.5) < tol);
  assert(rendertree_node_text_get_bold(node) == true);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);
  y += (rendertree_node_text_get_font_size(node) + rendertree_node_get_margin_bottom(node));

  // <h4>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is heading 4") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 18.5) < tol);
  assert(rendertree_node_text_get_bold(node) == true);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);
  y += (rendertree_node_text_get_font_size(node) + rendertree_node_get_margin_bottom(node));

  // <h5>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is heading 5") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 15.5) < tol);
  assert(rendertree_node_text_get_bold(node) == true);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);
  y += (rendertree_node_text_get_font_size(node) + rendertree_node_get_margin_bottom(node));

  // <h6>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is heading 6") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 12.0) < tol);
  assert(rendertree_node_text_get_bold(node) == true);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);
  y += (rendertree_node_text_get_font_size(node) + rendertree_node_get_margin_bottom(node));

  // <p>
  node = rendertree_next(&rendertree);
  y += rendertree_node_get_margin_top(node);
  assert(node->type == NODE_TYPE_TEXT);
  assert(strcmp(rendertree_node_text_get_value(node), "this is a paragraph") == 0);
  assert(strcmp(rendertree_node_text_get_font(node), "Times New Roman") == 0);
  assert(fabs(rendertree_node_text_get_font_size(node) - 18.5) < tol);
  assert(rendertree_node_text_get_bold(node) == false);
  assert(fabs(rendertree_node_get_x(node) - 0.00) < tol);
  assert(fabs(rendertree_node_get_y(node) - y) < tol);

  // end of iterator, last node is free'd
  node = rendertree_next(&rendertree);
  assert(node == NULL);

  rendertree_free(&rendertree);

  printf("\033[0;32mrendertree_test.c: All assertions passed\n\033[0;30m");
  return 0;
}
