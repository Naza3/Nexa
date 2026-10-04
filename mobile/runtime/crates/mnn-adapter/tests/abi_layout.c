#include <stddef.h>
#include "nexa_mnn.h"
_Static_assert(sizeof(nexa_mnn_v1_bytes)==16,"bytes");
_Static_assert(sizeof(nexa_mnn_v1_error)==528,"error");
_Static_assert(sizeof(nexa_mnn_v1_build)==184,"build");
_Static_assert(sizeof(nexa_mnn_v1_load_options)==120,"load");
_Static_assert(sizeof(nexa_mnn_v1_message)==32,"message");
_Static_assert(sizeof(nexa_mnn_v1_request)==80,"request");
_Static_assert(sizeof(nexa_mnn_v1_prepared_info)==24,"prepared");
_Static_assert(sizeof(nexa_mnn_v1_result)==32,"result");
_Static_assert(offsetof(nexa_mnn_v1_request,progress)==56,"progress offset");
_Static_assert(offsetof(nexa_mnn_v1_load_options,progress_user)==112,"userdata offset");
int main(void){return 0;}
