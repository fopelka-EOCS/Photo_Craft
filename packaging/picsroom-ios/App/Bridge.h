#ifndef PHOTOCRAFT_IOS_BRIDGE_H
#define PHOTOCRAFT_IOS_BRIDGE_H
// UTF-8 JSON in/out. Calls are serialized by Rust. Free each response exactly once.
char *pc_request(const char *request);
void pc_free(char *response);
#endif
