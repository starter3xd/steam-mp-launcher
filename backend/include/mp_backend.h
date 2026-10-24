#ifndef MP_BACKEND_H
#define MP_BACKEND_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MpBackend MpBackend;

/* Create/free the backend. NULL on allocation failure. */
MpBackend *mp_backend_new(void);
void mp_backend_free(MpBackend *backend);

/* Call one JSON command. The returned string is owned by the caller and must
 * be released with mp_backend_free_string(). Never returns NULL. */
char *mp_backend_call(MpBackend *backend, const char *json_request);
void mp_backend_free_string(char *text);

#ifdef __cplusplus
}
#endif

#endif
