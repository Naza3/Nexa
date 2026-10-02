#include <MNN/MNNDefine.h>
#include <cstdlib>
int main() {
  int side_effect = 0;
  MNN_PRINT("NEXA_MACRO_PRINT_CANARY %d", ++side_effect);
  MNN_ERROR("NEXA_MACRO_ERROR_CANARY %d", ++side_effect);
  if (side_effect != 0)
    std::abort();
}
