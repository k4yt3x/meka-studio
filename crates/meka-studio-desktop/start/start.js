// The app puts why it could not start in the address, as `#error=<message>`.
const status = document.getElementById('status');
const message = document.getElementById('message');
function showError() {
  const error = /^#error=(.*)$/.exec(location.hash);
  if (!error) return;
  message.textContent = decodeURIComponent(error[1].replace(/\+/g, ' '));
  status.className = 'error';
}
window.addEventListener('hashchange', showError);
showError();
