/* ThreadSanitizer driver for unitrisat-cdd (task P1.9; run through run.sh).
 * One-time init on the main thread, then THREADS threads compute hulls and
 * solve LPs concurrently on their own objects, as the crate does without a
 * lock. Exits nonzero if any result is wrong. */
#include <gmp.h>
#include <pthread.h>
#include <stdio.h>

typedef struct M M; typedef struct L L;
void ucdd_init(void);
M *ucdd_matrix_new(long, long, int);
void ucdd_matrix_free(M *);
long ucdd_matrix_rows(M *);
void ucdd_matrix_set(M *, long, long, mpq_srcptr);
void ucdd_matrix_set_objective(M *, int);
void ucdd_matrix_set_objective_entry(M *, long, mpq_srcptr);
M *ucdd_inequalities(M *, int *);
L *ucdd_lp_solve(M *, int *);
void ucdd_lp_free(L *);
int ucdd_lp_status(L *);

#define THREADS 8
#define ROUNDS 50
static int bad[THREADS];

static void *work(void *arg) {
  long t = (long)arg;
  mpq_t v; mpq_init(v);
  for (int r = 0; r < ROUNDS; r++) {
    M *g = ucdd_matrix_new(16, 5, 2);
    for (long i = 0; i < 16; i++) {
      mpq_set_si(v, 1, 1); ucdd_matrix_set(g, i, 0, v);
      for (long k = 0; k < 4; k++) { mpq_set_si(v, (i >> k) & 1, 1); ucdd_matrix_set(g, i, k + 1, v); }
    }
    int err; M *h = ucdd_inequalities(g, &err);
    if (h == NULL || err != 17 || ucdd_matrix_rows(h) != 8) bad[t]++;
    if (h) ucdd_matrix_free(h);
    ucdd_matrix_free(g);
    M *m = ucdd_matrix_new(14, 7, 1);
    for (long k = 0; k < 6; k++) { mpq_set_si(v, 1, 1); ucdd_matrix_set(m, k, k + 1, v); }
    for (long i = 0; i < 8; i++) {
      mpq_set_si(v, 7 + 3 * i, 1); ucdd_matrix_set(m, 6 + i, 0, v);
      for (long k = 0; k < 6; k++) { mpq_set_si(v, -((i * k + i + k) % 5 + 1), 1); ucdd_matrix_set(m, 6 + i, k + 1, v); }
    }
    ucdd_matrix_set_objective(m, 1);
    for (long k = 0; k < 7; k++) { mpq_set_si(v, k, 1); ucdd_matrix_set_objective_entry(m, k, v); }
    L *lp = ucdd_lp_solve(m, &err);
    if (lp == NULL || ucdd_lp_status(lp) != 1) bad[t]++;
    if (lp) ucdd_lp_free(lp);
    ucdd_matrix_free(m);
  }
  mpq_clear(v);
  return NULL;
}

int main(void) {
  ucdd_init();
  pthread_t th[THREADS];
  for (long t = 0; t < THREADS; t++) {
    if (pthread_create(&th[t], NULL, work, (void *)t) != 0) {
      fputs("ERROR: pthread_create failed\n", stderr);
      return 1;
    }
  }
  int total = 0;
  for (long t = 0; t < THREADS; t++) {
    if (pthread_join(th[t], NULL) != 0) {
      fputs("ERROR: pthread_join failed\n", stderr);
      return 1;
    }
    total += bad[t];
  }
  printf("wrong results: %d\n", total);
  return total != 0;
}
