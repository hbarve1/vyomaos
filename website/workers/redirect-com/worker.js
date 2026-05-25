export default {
	async fetch(request) {
		const url = new URL(request.url);
		return Response.redirect(`https://vyomaos.org${url.pathname}${url.search}`, 301);
	},
};
